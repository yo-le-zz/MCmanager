//! Détection des installations Java (Linux, macOS, Windows).
//!
//! Une application lancée depuis le Finder, un raccourci Windows ou un service n'hérite pas toujours du `PATH`
//! d'un terminal : `java` y reste introuvable alors qu'il est installé. On cherche donc aux emplacements usuels
//! (JAVA_HOME, `/usr/lib/jvm`, Homebrew, `/Library/Java/JavaVirtualMachines`, `Program Files`...) et on
//! interroge chaque candidat avec `-version`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct JavaInstall {
    pub path: String,
    /// Première ligne de `java -version`.
    pub version: String,
    /// Version majeure (8, 11, 17, 21...).
    pub major: Option<u32>,
}

/// Version majeure d'après la sortie de `java -version` (`"1.8.0_392"` -> 8, `"21.0.2"` -> 21, `"17"` -> 17).
pub fn parse_major(output: &str) -> Option<u32> {
    let start = output.find('"')? + 1;
    let rest = &output[start..];
    let token = &rest[..rest.find('"')?];
    let mut parts = token.split(|c: char| c == '.' || c == '_' || c == '-' || c == '+');
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

/// Version de Java conseillée pour une version de Minecraft : 8 jusqu'à 1.16, 17 de 1.17 à 1.20.4, 21 ensuite.
pub fn required_major(mc_version: &str) -> u32 {
    let mut it = mc_version.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let (major, minor, patch) = (it.next().unwrap_or(1), it.next().unwrap_or(0), it.next().unwrap_or(0));
    if major > 1 || minor > 20 || (minor == 20 && patch >= 5) {
        21
    } else if minor >= 17 {
        17
    } else {
        8
    }
}

fn exe_name() -> &'static str {
    if cfg!(windows) { "java.exe" } else { "java" }
}

/// Pour chaque sous-dossier `d` de `dir`, ajoute `d/<suffix>/java`.
fn expand(dir: &Path, suffix: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let mut p = e.path();
        if !p.is_dir() {
            continue;
        }
        for s in suffix {
            p.push(s);
        }
        p.push(exe_name());
        out.push(p);
    }
}

/// Emplacements candidats (existants), sans doublon.
pub fn candidates() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Ok(home) = std::env::var("JAVA_HOME") {
        v.push(Path::new(&home).join("bin").join(exe_name()));
    }
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            v.push(dir.join(exe_name()));
        }
    }
    let home = dirs::home_dir();
    #[cfg(target_os = "linux")]
    {
        for d in ["/usr/lib/jvm", "/usr/lib64/jvm", "/usr/java", "/opt/java", "/opt/jdk"] {
            expand(Path::new(d), &["bin"], &mut v);
        }
        if let Some(h) = &home {
            expand(&h.join(".sdkman/candidates/java"), &["bin"], &mut v);
            expand(&h.join(".jdks"), &["bin"], &mut v);
        }
    }
    #[cfg(target_os = "macos")]
    {
        expand(Path::new("/Library/Java/JavaVirtualMachines"), &["Contents", "Home", "bin"], &mut v);
        if let Some(h) = &home {
            expand(&h.join("Library/Java/JavaVirtualMachines"), &["Contents", "Home", "bin"], &mut v);
            expand(&h.join(".sdkman/candidates/java"), &["bin"], &mut v);
        }
        // Homebrew : Apple Silicon (/opt/homebrew) et Intel (/usr/local).
        for base in ["/opt/homebrew/opt", "/usr/local/opt"] {
            if let Ok(rd) = std::fs::read_dir(base) {
                for e in rd.flatten() {
                    if e.file_name().to_string_lossy().starts_with("openjdk") {
                        v.push(e.path().join("bin").join("java"));
                        v.push(e.path().join("libexec/openjdk.jdk/Contents/Home/bin/java"));
                    }
                }
            }
        }
    }
    #[cfg(windows)]
    {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Ok(pf) = std::env::var(var) {
                for vendor in ["Java", "Eclipse Adoptium", "Microsoft", "Zulu", "Amazon Corretto", "BellSoft", "Semeru"] {
                    expand(&Path::new(&pf).join(vendor), &["bin"], &mut v);
                }
            }
        }
    }
    let _ = &home;
    let mut seen = HashSet::new();
    v.into_iter().filter(|p| p.is_file()).filter(|p| seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))).collect()
}

/// Lance `java -version` (5 s au plus) et décrit l'installation.
pub async fn probe(path: &Path) -> Option<JavaInstall> {
    let out = tokio::time::timeout(Duration::from_secs(5), tokio::process::Command::new(path).arg("-version").output()).await.ok()?.ok()?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        return None;
    }
    let major = parse_major(&text);
    let version = text.lines().next().unwrap_or("").trim().to_string();
    major.map(|_| JavaInstall { path: path.to_string_lossy().to_string(), version, major })
}

/// Toutes les installations utilisables, la plus récente d'abord.
pub async fn detect() -> Vec<JavaInstall> {
    let mut out = Vec::new();
    for c in candidates() {
        if let Some(j) = probe(&c).await {
            out.push(j);
        }
    }
    out.sort_by(|a, b| b.major.cmp(&a.major).then_with(|| a.path.cmp(&b.path)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_java_versions() {
        assert_eq!(parse_major("openjdk version \"21.0.2\" 2024-01-16"), Some(21));
        assert_eq!(parse_major("java version \"1.8.0_392\"\nJava(TM) SE Runtime"), Some(8));
        assert_eq!(parse_major("openjdk version \"17\" 2021-09-14"), Some(17));
        assert_eq!(parse_major("openjdk version \"11.0.22\" 2024-01-16"), Some(11));
        assert_eq!(parse_major("openjdk version \"22-ea\" 2024-03-19"), Some(22));
        assert_eq!(parse_major("pas du java"), None);
        assert_eq!(parse_major("version \"abc\""), None);
    }

    #[test]
    fn minecraft_java_requirements() {
        for (mc, j) in [("1.8.9", 8), ("1.12.2", 8), ("1.16.5", 8), ("1.17", 17), ("1.18.2", 17), ("1.20.1", 17), ("1.20.4", 17), ("1.20.5", 21), ("1.20.6", 21), ("1.21", 21), ("1.21.4", 21), ("26.1", 21), ("n'importe quoi", 8)] {
            assert_eq!(required_major(mc), j, "{mc}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probes_and_expands_fake_installs() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mcm-java-{}", uuid::Uuid::new_v4()));
        let good = dir.join("jdk-21/bin");
        let bad = dir.join("jdk-broken/bin");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::create_dir_all(&bad).unwrap();
        let script = |p: &Path, body: &str| {
            std::fs::write(p, body).unwrap();
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        script(&good.join("java"), "#!/bin/sh\necho 'openjdk version \"21.0.2\" 2024-01-16' >&2\n");
        script(&bad.join("java"), "#!/bin/sh\nexit 1\n");
        let mut found = Vec::new();
        expand(&dir, &["bin"], &mut found);
        assert_eq!(found.len(), 2);
        let ok = probe(&good.join("java")).await.expect("java valide");
        assert_eq!((ok.major, ok.version.as_str()), (Some(21), "openjdk version \"21.0.2\" 2024-01-16"));
        assert!(probe(&bad.join("java")).await.is_none(), "un java en échec est ignoré");
        assert!(probe(&dir.join("absent/java")).await.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
