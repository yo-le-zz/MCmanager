//! Installation de MCManager comme service qui démarre avec la machine : systemd (Linux), launchd (macOS), tâche planifiée (Windows).
//!
//! `mcmanager-server --install-service` crée le service **de l'utilisateur courant** (pas besoin de droits administrateur
//! sous Linux et macOS), `--uninstall-service` le retire, `--service-status` l'interroge. Les fichiers générés sont
//! construits par des fonctions pures (testées) ; seule l'exécution des commandes système dépend de la machine.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const LABEL: &str = "io.github.yo-le-zz.mcmanager-server";
pub const UNIT_NAME: &str = "mcmanager-server.service";
pub const TASK_NAME: &str = "MCManager Server";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
}

impl Platform {
    pub fn current() -> Option<Platform> {
        match std::env::consts::OS {
            "linux" => Some(Platform::Linux),
            "macos" => Some(Platform::MacOs),
            "windows" => Some(Platform::Windows),
            _ => None,
        }
    }
}

/// Échappe une valeur pour un fichier plist (XML).
pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Plist launchd (agent utilisateur) : démarre au login, relancé s'il s'arrête, journaux dans `log_dir`.
pub fn launchd_plist(exe: &Path, args: &[&str], envs: &[(&str, &str)], log_dir: &Path) -> String {
    let mut prog = format!("    <string>{}</string>\n", xml_escape(&exe.to_string_lossy()));
    for a in args {
        prog.push_str(&format!("    <string>{}</string>\n", xml_escape(a)));
    }
    let env: String = envs.iter().map(|(k, v)| format!("    <key>{}</key><string>{}</string>\n", xml_escape(k), xml_escape(v))).collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{prog}  </array>
  <key>EnvironmentVariables</key>
  <dict>
{env}  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Background</string>
  <key>StandardOutPath</key><string>{out}</string>
  <key>StandardErrorPath</key><string>{err}</string>
</dict>
</plist>
"#,
        out = xml_escape(&log_dir.join("mcmanager-server.log").to_string_lossy()),
        err = xml_escape(&log_dir.join("mcmanager-server.err.log").to_string_lossy()),
    )
}

/// Guillemets d'une valeur de ligne `ExecStart` systemd.
fn systemd_quote(s: &str) -> String {
    if s.chars().any(|c| c.is_whitespace() || c == '"' || c == '\\') { format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")) } else { s.to_string() }
}

/// Unité systemd **utilisateur** (`systemctl --user`), arrêt propre long : les serveurs Minecraft ont le temps de sauvegarder.
pub fn systemd_user_unit(exe: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let exec = std::iter::once(exe.to_string_lossy().to_string()).chain(args.iter().map(|a| a.to_string())).map(|a| systemd_quote(&a)).collect::<Vec<_>>().join(" ");
    let env: String = envs.iter().map(|(k, v)| format!("Environment={}\n", systemd_quote(&format!("{k}={v}")))).collect();
    format!(
        "[Unit]\nDescription=MCManager server (Minecraft)\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\nExecStart={exec}\n{env}Restart=on-failure\nRestartSec=5\n# Arrêt propre : les serveurs Minecraft sauvegardent leur monde avant de s'éteindre.\nTimeoutStopSec=120\nKillMode=mixed\n\n[Install]\nWantedBy=default.target\n"
    )
}

/// Arguments de `schtasks /Create` : tâche lancée à l'ouverture de session de l'utilisateur, relancée en cas d'échec.
pub fn schtasks_create_args(exe: &Path, args: &[&str]) -> Vec<String> {
    let mut tr = format!("\"{}\"", exe.to_string_lossy());
    for a in args {
        tr.push(' ');
        tr.push_str(a);
    }
    ["/Create", "/TN", TASK_NAME, "/TR", &tr, "/SC", "ONLOGON", "/RL", "LIMITED", "/F"].iter().map(|s| s.to_string()).collect()
}

pub fn schtasks_delete_args() -> Vec<String> {
    ["/Delete", "/TN", TASK_NAME, "/F"].iter().map(|s| s.to_string()).collect()
}

fn home() -> Result<PathBuf> {
    dirs::home_dir().context("dossier personnel introuvable")
}

pub fn systemd_unit_path() -> Result<PathBuf> {
    Ok(home()?.join(".config/systemd/user").join(UNIT_NAME))
}

pub fn launchd_plist_path() -> Result<PathBuf> {
    Ok(home()?.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
}

async fn run(cmd: &str, args: &[String]) -> Result<String> {
    let out = tokio::process::Command::new(cmd).args(args).output().await.with_context(|| format!("impossible de lancer {cmd}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).trim().to_string();
    if !out.status.success() {
        anyhow::bail!("{cmd} {} : {text}", args.join(" "));
    }
    Ok(text)
}

fn sv(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// Installe le service pour cette machine. `exe` : chemin de `mcmanager-server`. Renvoie un message d'information.
pub async fn install(exe: &Path) -> Result<String> {
    let platform = Platform::current().context("système non pris en charge")?;
    let args = ["--daemon"];
    match platform {
        Platform::Linux => {
            let path = systemd_unit_path()?;
            tokio::fs::create_dir_all(path.parent().context("chemin")?).await?;
            tokio::fs::write(&path, systemd_user_unit(exe, &args, &[])).await?;
            run("systemctl", &sv(&["--user", "daemon-reload"])).await?;
            run("systemctl", &sv(&["--user", "enable", "--now", UNIT_NAME])).await?;
            // Sans « linger », un service utilisateur s'arrête à la déconnexion : on le garde actif au démarrage de la machine.
            let linger = run("loginctl", &sv(&["enable-linger"])).await.is_ok();
            Ok(format!("Service installé : {}{}\nÉtat : systemctl --user status {UNIT_NAME}  ·  journal : journalctl --user -u {UNIT_NAME}", path.display(), if linger { "\nDémarrage avec la machine activé (linger)." } else { "\nPour qu'il démarre sans ouvrir de session : sudo loginctl enable-linger $USER" }))
        }
        Platform::MacOs => {
            let path = launchd_plist_path()?;
            let logs = home()?.join("Library/Logs/MCManager");
            tokio::fs::create_dir_all(&logs).await?;
            tokio::fs::create_dir_all(path.parent().context("chemin")?).await?;
            let path_env = "/opt/homebrew/bin:/opt/homebrew/opt/openjdk/bin:/usr/local/bin:/usr/bin:/bin";
            tokio::fs::write(&path, launchd_plist(exe, &args, &[("PATH", path_env)], &logs)).await?;
            let _ = run("launchctl", &sv(&["unload", &path.to_string_lossy()])).await; // déjà chargé : on recharge
            run("launchctl", &sv(&["load", "-w", &path.to_string_lossy()])).await?;
            Ok(format!("Service installé : {}\nJournaux : {}", path.display(), logs.display()))
        }
        Platform::Windows => {
            run("schtasks", &schtasks_create_args(exe, &args)).await?;
            let _ = run("schtasks", &sv(&["/Run", "/TN", TASK_NAME])).await;
            Ok(format!("Tâche planifiée « {TASK_NAME} » créée : MCManager démarre à l'ouverture de session et tourne en arrière-plan.\nAfficher : schtasks /Query /TN \"{TASK_NAME}\"  ·  retirer : mcmanager-server --uninstall-service"))
        }
    }
}

pub async fn uninstall() -> Result<String> {
    match Platform::current().context("système non pris en charge")? {
        Platform::Linux => {
            let _ = run("systemctl", &sv(&["--user", "disable", "--now", UNIT_NAME])).await;
            let path = systemd_unit_path()?;
            let existed = path.exists();
            let _ = tokio::fs::remove_file(&path).await;
            let _ = run("systemctl", &sv(&["--user", "daemon-reload"])).await;
            Ok(if existed { "Service retiré.".into() } else { "Aucun service installé.".into() })
        }
        Platform::MacOs => {
            let path = launchd_plist_path()?;
            let existed = path.exists();
            let _ = run("launchctl", &sv(&["unload", "-w", &path.to_string_lossy()])).await;
            let _ = tokio::fs::remove_file(&path).await;
            Ok(if existed { "Service retiré.".into() } else { "Aucun service installé.".into() })
        }
        Platform::Windows => match run("schtasks", &schtasks_delete_args()).await {
            Ok(_) => Ok("Tâche planifiée supprimée.".into()),
            Err(_) => Ok("Aucune tâche planifiée à supprimer.".into()),
        },
    }
}

pub async fn status() -> Result<String> {
    match Platform::current().context("système non pris en charge")? {
        Platform::Linux => Ok(run("systemctl", &sv(&["--user", "status", UNIT_NAME, "--no-pager"])).await.unwrap_or_else(|e| format!("Service absent ou arrêté ({e})"))),
        Platform::MacOs => Ok(run("launchctl", &sv(&["list", LABEL])).await.unwrap_or_else(|_| "Service non chargé.".into())),
        Platform::Windows => Ok(run("schtasks", &sv(&["/Query", "/TN", TASK_NAME, "/FO", "LIST"])).await.unwrap_or_else(|_| "Tâche absente.".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launchd_plist_is_valid_xml_and_escapes_values() {
        let p = launchd_plist(Path::new("/Applications/My <App>/mcmanager-server"), &["--daemon"], &[("PATH", "/a:/b"), ("MCMANAGER_NAME", "Jean & \"Co\"")], Path::new("/Users/x/Library/Logs/MCManager"));
        assert!(p.contains(&format!("<string>{LABEL}</string>")) && p.contains("<string>--daemon</string>"));
        assert!(p.contains("My &lt;App&gt;") && p.contains("Jean &amp; &quot;Co&quot;"), "{p}");
        assert!(p.contains("<key>KeepAlive</key><true/>") && p.contains("<key>RunAtLoad</key><true/>"));
        assert!(p.contains("mcmanager-server.err.log"));
        // Bien formé : mêmes nombres de balises ouvrantes et fermantes pour les éléments structurants.
        for tag in ["dict", "array", "plist"] {
            assert_eq!(p.matches(&format!("<{tag}>")).count() + p.matches(&format!("<{tag} ")).count(), p.matches(&format!("</{tag}>")).count(), "{tag}");
        }
    }

    #[test]
    fn systemd_unit_quotes_and_waits_for_clean_stop() {
        let u = systemd_user_unit(Path::new("/home/jean/Mon Dossier/mcmanager-server"), &["--daemon"], &[("MCMANAGER_PORT", "7777"), ("X", "a b")]);
        assert!(u.contains("ExecStart=\"/home/jean/Mon Dossier/mcmanager-server\" --daemon\n"), "{u}");
        assert!(u.contains("Environment=MCMANAGER_PORT=7777\n") && u.contains("Environment=\"X=a b\"\n"));
        assert!(u.contains("TimeoutStopSec=120") && u.contains("Restart=on-failure") && u.contains("WantedBy=default.target"));
        let simple = systemd_user_unit(Path::new("/usr/bin/mcmanager-server"), &["--daemon"], &[]);
        assert!(simple.contains("ExecStart=/usr/bin/mcmanager-server --daemon\n"));
    }

    #[test]
    fn schtasks_arguments() {
        let a = schtasks_create_args(Path::new(r"C:\Program Files\MCManager\mcmanager-server.exe"), &["--daemon"]);
        assert_eq!(a[..2], ["/Create", "/TN"]);
        assert!(a.contains(&"ONLOGON".to_string()) && a.contains(&"/F".to_string()));
        let tr = &a[a.iter().position(|x| x == "/TR").unwrap() + 1];
        assert_eq!(tr, r#""C:\Program Files\MCManager\mcmanager-server.exe" --daemon"#, "chemin avec espaces entre guillemets");
        assert_eq!(schtasks_delete_args(), ["/Delete", "/TN", TASK_NAME, "/F"]);
    }

    #[test]
    fn paths_and_platform() {
        assert!(systemd_unit_path().unwrap().ends_with(".config/systemd/user/mcmanager-server.service"));
        assert!(launchd_plist_path().unwrap().to_string_lossy().ends_with(&format!("Library/LaunchAgents/{LABEL}.plist")));
        assert!(Platform::current().is_some());
        assert_eq!(xml_escape("a&b<c>\"d"), "a&amp;b&lt;c&gt;&quot;d");
    }
}
