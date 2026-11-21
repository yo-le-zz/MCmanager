//! Packs et mondes du serveur Bedrock officiel : add-ons (`.mcpack`, `.mcaddon`), packs de ressources, mondes (`.mcworld`).
//!
//! Un pack est un dossier avec `manifest.json` dans `behavior_packs/` (comportements, scripts) ou `resource_packs/`
//! (textures, sons). Il n'est **actif** que s'il est déclaré dans `worlds/<monde>/world_behavior_packs.json` ou
//! `world_resource_packs.json` : c'est ce que fait « Activer ». Les manifestes réels contiennent souvent des commentaires
//! et des virgules finales (JSON « libre ») : ils sont tolérés.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const BEHAVIOR_DIR: &str = "behavior_packs";
pub const RESOURCE_DIR: &str = "resource_packs";

/// Retire commentaires `//` et `/* */` (hors chaînes) et virgules finales d'un JSON « libre ».
pub fn clean_json(src: &str) -> String {
    let b: Vec<char> = src.trim_start_matches('\u{feff}').chars().collect();
    let mut out = String::with_capacity(b.len());
    let (mut i, mut in_str, mut esc) = (0, false, false);
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(c);
            if esc { esc = false } else if c == '\\' { esc = true } else if c == '"' { in_str = false }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' { i += 1 }
            continue;
        } else if c == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') { i += 1 }
            i += 2;
            continue;
        } else {
            out.push(c);
        }
        i += 1;
    }
    // Virgules finales : `,` suivie (après espaces) de `}` ou `]`, hors chaînes.
    let chars: Vec<char> = out.chars().collect();
    let mut res = String::with_capacity(chars.len());
    let (mut in_str, mut esc) = (false, false);
    for (idx, &c) in chars.iter().enumerate() {
        if in_str {
            res.push(c);
            if esc { esc = false } else if c == '\\' { esc = true } else if c == '"' { in_str = false }
            continue;
        }
        if c == '"' { in_str = true; res.push(c); continue }
        if c == ',' {
            let next = chars[idx + 1..].iter().find(|x| !x.is_whitespace());
            if matches!(next, Some('}') | Some(']')) { continue }
        }
        res.push(c);
    }
    res
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Manifest {
    pub uuid: String,
    pub name: String,
    pub description: String,
    pub version: [u32; 3],
    /// `behavior` | `resource` | `skin`
    pub kind: String,
    pub dependencies: Vec<String>,
}

fn parse_version(v: &Value) -> [u32; 3] {
    match v {
        Value::Array(a) => [0, 1, 2].map(|i| a.get(i).and_then(|x| x.as_u64()).unwrap_or(0) as u32),
        Value::String(s) => {
            let mut it = s.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
            [it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0)]
        }
        _ => [1, 0, 0],
    }
}

pub fn parse_manifest(text: &str) -> Result<Manifest> {
    let j: Value = serde_json::from_str(&clean_json(text)).context("manifest.json illisible")?;
    let header = &j["header"];
    let uuid = header["uuid"].as_str().context("manifest sans identifiant (header.uuid)")?.to_ascii_lowercase();
    if uuid::Uuid::parse_str(&uuid).is_err() {
        anyhow::bail!("identifiant de pack invalide : {uuid}");
    }
    let types: Vec<&str> = j["modules"].as_array().map(|m| m.iter().filter_map(|x| x["type"].as_str()).collect()).unwrap_or_default();
    let kind = if types.iter().any(|t| matches!(*t, "data" | "script" | "client_data" | "interface")) { "behavior" }
        else if types.contains(&"resources") { "resource" }
        else if types.contains(&"skin_pack") { "skin" }
        else { anyhow::bail!("type de pack non pris en charge ({})", types.join(", ")) };
    Ok(Manifest {
        uuid,
        name: header["name"].as_str().unwrap_or("Pack sans nom").to_string(),
        description: header["description"].as_str().unwrap_or("").to_string(),
        version: parse_version(&header["version"]),
        kind: kind.to_string(),
        dependencies: j["dependencies"].as_array().map(|d| d.iter().filter_map(|x| x["uuid"].as_str().map(|s| s.to_ascii_lowercase())).collect()).unwrap_or_default(),
    })
}

/// Nom de dossier sûr pour un pack.
pub fn safe_dir_name(name: &str, uuid: &str) -> String {
    let base: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' }).collect::<String>().trim_matches('_').chars().take(40).collect();
    let short: String = uuid.chars().filter(|c| c.is_ascii_hexdigit()).take(8).collect();
    format!("{}_{short}", if base.is_empty() { "pack".to_string() } else { base })
}

fn pack_dir_for(kind: &str) -> &'static str {
    if kind == "behavior" { BEHAVIOR_DIR } else { RESOURCE_DIR } // les packs de skins vont avec les ressources
}

fn world_file(folder: &Path, level: &str, kind: &str) -> PathBuf {
    folder.join("worlds").join(level).join(if kind == "behavior" { "world_behavior_packs.json" } else { "world_resource_packs.json" })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActiveEntry {
    pub pack_id: String,
    pub version: Vec<u32>,
}

fn read_active(folder: &Path, level: &str, kind: &str) -> Vec<ActiveEntry> {
    std::fs::read_to_string(world_file(folder, level, kind)).ok().and_then(|s| serde_json::from_str(&clean_json(&s)).ok()).unwrap_or_default()
}

fn write_active(folder: &Path, level: &str, kind: &str, list: &[ActiveEntry]) -> Result<()> {
    let p = world_file(folder, level, kind);
    std::fs::create_dir_all(p.parent().context("chemin")?)?;
    std::fs::write(p, serde_json::to_string_pretty(list)?)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PackInfo {
    pub uuid: String,
    pub name: String,
    pub description: String,
    pub version: [u32; 3],
    pub kind: String,
    pub dir: String,
    /// Packs livrés avec le serveur (`vanilla`, `chemistry`, `editor`...) : affichés mais non supprimables.
    pub builtin: bool,
    pub active: bool,
    pub size_bytes: u64,
}

fn dir_size(p: &Path) -> u64 {
    walkdir::WalkDir::new(p).into_iter().flatten().filter(|e| e.file_type().is_file()).filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()
}

fn is_builtin(dir: &str) -> bool {
    matches!(dir, "vanilla" | "chemistry" | "editor" | "experimental_creator_cameras" | "experimental_custom_ui" | "vanilla_gametest" | "debug_pack")
        || dir.starts_with("vanilla") || dir.starts_with("experimental")
}

pub fn list_packs(folder: &Path, level: &str) -> Vec<PackInfo> {
    let mut out = Vec::new();
    for root in [BEHAVIOR_DIR, RESOURCE_DIR] {
        let Ok(rd) = std::fs::read_dir(folder.join(root)) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(text) = std::fs::read_to_string(p.join("manifest.json")) else { continue };
            let Ok(m) = parse_manifest(&text) else { continue };
            let dir = e.file_name().to_string_lossy().to_string();
            let active = read_active(folder, level, pack_kind_for_dir(root)).iter().any(|a| a.pack_id == m.uuid);
            out.push(PackInfo { builtin: is_builtin(&dir), uuid: m.uuid, name: m.name, description: m.description, version: m.version, kind: m.kind, dir, active, size_bytes: dir_size(&p) });
        }
    }
    out.sort_by(|a, b| a.builtin.cmp(&b.builtin).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

fn pack_kind_for_dir(root: &str) -> &'static str {
    if root == BEHAVIOR_DIR { "behavior" } else { "resource" }
}

/// Active ou désactive un pack (et, à l'activation, ses dépendances installées) dans le monde donné.
pub fn set_active(folder: &Path, level: &str, uuid: &str, on: bool) -> Result<()> {
    fn go(folder: &Path, level: &str, uuid: &str, on: bool, depth: u8, installed: &[PackInfo]) -> Result<()> {
        let Some(p) = installed.iter().find(|p| p.uuid == uuid) else {
            if depth == 0 { anyhow::bail!("pack introuvable") }
            return Ok(()); // dépendance non installée (pack du jeu, par exemple) : ignorée
        };
        let kind = if p.kind == "behavior" { "behavior" } else { "resource" };
        let mut list = read_active(folder, level, kind);
        list.retain(|a| a.pack_id != uuid);
        if on {
            list.push(ActiveEntry { pack_id: uuid.to_string(), version: p.version.to_vec() });
        }
        write_active(folder, level, kind, &list)?;
        if on && depth < 3 {
            let root = if p.kind == "behavior" { BEHAVIOR_DIR } else { RESOURCE_DIR };
            if let Ok(text) = std::fs::read_to_string(folder.join(root).join(&p.dir).join("manifest.json")) {
                if let Ok(m) = parse_manifest(&text) {
                    for dep in m.dependencies {
                        go(folder, level, &dep, true, depth + 1, installed)?;
                    }
                }
            }
        }
        Ok(())
    }
    go(folder, level, uuid, on, 0, &list_packs(folder, level))
}

/// Installe un `.mcpack` (un pack) ou un `.mcaddon` (plusieurs packs, éventuellement des `.mcpack` imbriqués).
/// `activate` : déclare aussi les packs dans le monde. Remplace un pack de même identifiant (mise à jour).
pub fn install_archive(folder: &Path, level: &str, archive: &Path, activate: bool) -> Result<Vec<PackInfo>> {
    let work = std::env::temp_dir().join(format!("mcm-packs-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work)?;
    let res = (|| -> Result<Vec<PackInfo>> {
        let mut roots = Vec::new();
        extract_collect(archive, &work.join("a"), 0, &mut roots)?;
        if roots.is_empty() {
            anyhow::bail!("aucun pack (manifest.json) trouvé dans cette archive");
        }
        let mut installed = Vec::new();
        for r in roots {
            let m = parse_manifest(&std::fs::read_to_string(r.join("manifest.json"))?)?;
            let root_dir = pack_dir_for(&m.kind);
            // Même identifiant déjà installé (ailleurs ou sous un autre nom) : on le remplace.
            for old in list_packs(folder, level).into_iter().filter(|p| p.uuid == m.uuid && !p.builtin) {
                let _ = std::fs::remove_dir_all(folder.join(if old.kind == "behavior" { BEHAVIOR_DIR } else { RESOURCE_DIR }).join(&old.dir));
            }
            let dest = folder.join(root_dir).join(safe_dir_name(&m.name, &m.uuid));
            copy_dir(&r, &dest)?;
            installed.push(m);
        }
        for m in &installed {
            if activate && m.kind != "skin" {
                set_active(folder, level, &m.uuid, true)?;
            }
        }
        let ids: Vec<&str> = installed.iter().map(|m| m.uuid.as_str()).collect();
        Ok(list_packs(folder, level).into_iter().filter(|p| ids.contains(&p.uuid.as_str())).collect())
    })();
    let _ = std::fs::remove_dir_all(&work);
    res
}

/// Extrait l'archive puis repère les dossiers de pack ; descend dans les `.mcpack`/`.zip` imbriqués (1 niveau).
fn extract_collect(archive: &Path, dest: &Path, depth: u8, roots: &mut Vec<PathBuf>) -> Result<()> {
    crate::files::import_zip_reader(dest, "", std::io::BufReader::new(std::fs::File::open(archive)?), 5_000, 2 * 1024 * 1024 * 1024).context("archive de pack invalide")?;
    if dest.join("manifest.json").is_file() {
        roots.push(dest.to_path_buf());
        return Ok(());
    }
    for e in walkdir::WalkDir::new(dest).max_depth(3).into_iter().flatten() {
        if e.file_type().is_file() && e.file_name() == "manifest.json" {
            if let Some(p) = e.path().parent() {
                if !roots.iter().any(|r| p.starts_with(r)) {
                    roots.push(p.to_path_buf());
                }
            }
        }
    }
    if roots.is_empty() && depth == 0 {
        let nested: Vec<PathBuf> = walkdir::WalkDir::new(dest).max_depth(2).into_iter().flatten().filter(|e| e.file_type().is_file()).map(|e| e.into_path())
            .filter(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("mcpack") | Some("zip"))).collect();
        for (i, n) in nested.iter().enumerate() {
            extract_collect(n, &dest.join(format!("_nested{i}")), 1, roots)?;
        }
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    for e in walkdir::WalkDir::new(from).into_iter().flatten() {
        let rel = e.path().strip_prefix(from)?;
        if rel.components().any(|c| c.as_os_str().to_string_lossy().starts_with("_nested")) {
            continue;
        }
        let target = to.join(rel);
        if e.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if e.file_type().is_file() {
            if let Some(p) = target.parent() { std::fs::create_dir_all(p)?; }
            std::fs::copy(e.path(), &target)?;
        }
    }
    Ok(())
}

/// Supprime un pack (jamais ceux livrés avec le serveur) et le retire des mondes.
pub fn remove_pack(folder: &Path, uuid: &str) -> Result<bool> {
    let Some(p) = list_packs(folder, "").into_iter().find(|p| p.uuid == uuid) else { return Ok(false) };
    if p.builtin {
        anyhow::bail!("ce pack est livré avec le serveur : il ne peut pas être supprimé");
    }
    std::fs::remove_dir_all(folder.join(if p.kind == "behavior" { BEHAVIOR_DIR } else { RESOURCE_DIR }).join(&p.dir))?;
    if let Ok(rd) = std::fs::read_dir(folder.join("worlds")) {
        for w in rd.flatten().filter(|w| w.path().is_dir()) {
            let level = w.file_name().to_string_lossy().to_string();
            for kind in ["behavior", "resource"] {
                let mut list = read_active(folder, &level, kind);
                let before = list.len();
                list.retain(|a| a.pack_id != uuid);
                if list.len() != before {
                    write_active(folder, &level, kind, &list)?;
                }
            }
        }
    }
    Ok(true)
}

// ───────────────────────── mondes ─────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorldInfo {
    pub name: String,
    pub size_bytes: u64,
    pub active: bool,
}

pub fn list_worlds(folder: &Path, active_level: &str) -> Vec<WorldInfo> {
    let Ok(rd) = std::fs::read_dir(folder.join("worlds")) else { return vec![] };
    let mut v: Vec<WorldInfo> = rd.flatten().filter(|e| e.path().is_dir()).map(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        WorldInfo { active: name == active_level, size_bytes: dir_size(&e.path()), name }
    }).collect();
    v.sort_by(|a, b| b.active.cmp(&a.active).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    v
}

/// Nom de dossier de monde sûr (pas de chemin, 1 à 60 caractères).
pub fn valid_world_name(n: &str) -> bool {
    let t = n.trim();
    !t.is_empty() && t.chars().count() <= 60 && t != "." && t != ".." && !t.contains(['/', '\\', '\0', ':']) && !t.starts_with('.')
}

/// Importe un `.mcworld` (zip dont la racine contient `db/`, `level.dat`, `levelname.txt`) sous `worlds/<nom>`.
/// Refuse d'écraser un monde existant (sauf `overwrite`). Renvoie le nom du monde.
pub fn import_world(folder: &Path, archive: &Path, fallback_name: &str, overwrite: bool) -> Result<String> {
    let work = std::env::temp_dir().join(format!("mcm-world-{}", uuid::Uuid::new_v4()));
    let res = (|| -> Result<String> {
        crate::files::import_zip_reader(&work, "", std::io::BufReader::new(std::fs::File::open(archive)?), 500_000, 64 * 1024 * 1024 * 1024).context("archive de monde invalide")?;
        // Racine : le dossier qui contient level.dat ou db/ (à la racine, ou un seul niveau plus bas).
        let is_root = |p: &Path| p.join("level.dat").is_file() || p.join("db").is_dir();
        let root = if is_root(&work) { work.clone() } else {
            std::fs::read_dir(&work)?.flatten().map(|e| e.path()).find(|p| p.is_dir() && is_root(p)).context("ce n'est pas un monde Bedrock (level.dat ou db/ introuvable)")?
        };
        let name = std::fs::read_to_string(root.join("levelname.txt")).ok().map(|s| s.trim().to_string()).filter(|s| valid_world_name(s))
            .or_else(|| Some(fallback_name.trim().to_string()).filter(|s| valid_world_name(s))).context("nom de monde introuvable : renommez le fichier")?;
        let dest = folder.join("worlds").join(&name);
        if dest.exists() {
            if !overwrite { anyhow::bail!("le monde « {name} » existe déjà (cochez « remplacer » pour l'écraser)") }
            std::fs::remove_dir_all(&dest)?;
        }
        copy_dir(&root, &dest)?;
        Ok(name)
    })();
    let _ = std::fs::remove_dir_all(&work);
    res
}

pub fn remove_world(folder: &Path, name: &str, active_level: &str) -> Result<bool> {
    if !valid_world_name(name) { anyhow::bail!("nom de monde invalide") }
    if name == active_level { anyhow::bail!("ce monde est actif : changez de monde (level-name) avant de le supprimer") }
    let p = folder.join("worlds").join(name);
    if !p.is_dir() { return Ok(false) }
    std::fs::remove_dir_all(p)?;
    Ok(true)
}

pub fn summary(folder: &Path, level: &str) -> Value {
    json!({ "packs": list_packs(folder, level), "worlds": list_worlds(folder, level), "level": level })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const BP: &str = r#"{
        // pack de comportement avec commentaires et virgule finale
        "format_version": 2,
        "header": { "name": "Mon Add-on", "description": "Des mobs", "uuid": "11111111-1111-4111-8111-111111111111", "version": [1, 2, 3], "min_engine_version": [1, 20, 0], },
        "modules": [ { "type": "data", "uuid": "22222222-2222-4222-8222-222222222222", "version": [1, 0, 0], }, ],
        "dependencies": [ { "uuid": "33333333-3333-4333-8333-333333333333", "version": [1, 2, 3] } ] /* fin */
    }"#;
    const RP: &str = r#"{ "format_version": 2, "header": { "name": "Textures", "uuid": "33333333-3333-4333-8333-333333333333", "version": "1.2.3" }, "modules": [ { "type": "resources", "uuid": "44444444-4444-4444-8444-444444444444", "version": [1,0,0] } ] }"#;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (n, d) in files {
            z.start_file(*n, zip::write::FileOptions::default()).unwrap();
            z.write_all(d).unwrap();
        }
        z.finish().unwrap().into_inner()
    }
    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("mcm-packtest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn lenient_json_and_manifest_parsing() {
        assert_eq!(clean_json(r#"{"a": "http://x//y", /* c */ "b": [1, 2,], // fin
        }"#).replace([' ', '\n'], ""), r#"{"a":"http://x//y","b":[1,2]}"#);
        assert_eq!(clean_json("\u{feff}{\"s\": \"a,}\"}"), "{\"s\": \"a,}\"}", "les virgules dans une chaîne restent");
        let m = parse_manifest(BP).unwrap();
        assert_eq!((m.kind.as_str(), m.version, m.name.as_str()), ("behavior", [1, 2, 3], "Mon Add-on"));
        assert_eq!(m.dependencies, ["33333333-3333-4333-8333-333333333333"]);
        let r = parse_manifest(RP).unwrap();
        assert_eq!((r.kind.as_str(), r.version), ("resource", [1, 2, 3]), "version en chaîne acceptée");
        assert!(parse_manifest("{}").is_err() && parse_manifest("pas du json").is_err());
        assert!(parse_manifest(r#"{"header":{"uuid":"pas-un-uuid"},"modules":[{"type":"data"}]}"#).is_err());
        assert!(parse_manifest(r#"{"header":{"uuid":"11111111-1111-4111-8111-111111111111"},"modules":[{"type":"truc"}]}"#).is_err());
        assert_eq!(safe_dir_name("Mon Add-on/../x", "11111111-aaaa"), "Mon_Add-on____x_11111111");
        assert_eq!(safe_dir_name("///", "abcdef12-0000"), "pack_abcdef12");
    }

    #[test]
    fn install_mcpack_and_mcaddon_activate_with_dependencies_and_update() {
        let d = tmp();
        std::fs::create_dir_all(d.join("worlds/Mon monde")).unwrap();
        // Addon : deux packs dans des dossiers.
        let addon = zip_of(&[("BP/manifest.json", BP.as_bytes()), ("BP/entities/a.json", b"{}"), ("RP/manifest.json", RP.as_bytes()), ("RP/textures/t.png", b"PNG")]);
        let a = d.join("x.mcaddon");
        std::fs::write(&a, addon).unwrap();
        let ins = install_archive(&d, "Mon monde", &a, true).unwrap();
        assert_eq!(ins.len(), 2);
        assert!(ins.iter().all(|p| p.active), "packs actifs dans le monde : {ins:?}");
        let bp = ins.iter().find(|p| p.kind == "behavior").unwrap();
        assert!(d.join(BEHAVIOR_DIR).join(&bp.dir).join("entities/a.json").is_file());
        let active: Vec<ActiveEntry> = serde_json::from_str(&std::fs::read_to_string(d.join("worlds/Mon monde/world_behavior_packs.json")).unwrap()).unwrap();
        assert_eq!(active, vec![ActiveEntry { pack_id: "11111111-1111-4111-8111-111111111111".into(), version: vec![1, 2, 3] }]);

        // Désactiver puis réactiver : le pack de comportement entraîne sa dépendance (le pack de ressources).
        set_active(&d, "Mon monde", &bp.uuid, false).unwrap();
        set_active(&d, "Mon monde", "33333333-3333-4333-8333-333333333333", false).unwrap();
        assert!(list_packs(&d, "Mon monde").iter().all(|p| !p.active));
        set_active(&d, "Mon monde", &bp.uuid, true).unwrap();
        assert!(list_packs(&d, "Mon monde").iter().all(|p| p.active), "dépendance activée avec lui");
        assert!(set_active(&d, "Mon monde", "99999999-9999-4999-8999-999999999999", true).is_err());

        // Un .mcpack seul, même identifiant : remplace l'ancien (mise à jour), sans doublon.
        let upd = BP.replace("[1, 2, 3], \"min", "[2, 0, 0], \"min").replace("\"version\": [1, 2, 3] } ]", "\"version\": [1, 2, 3] } ]");
        let p = d.join("up.mcpack");
        std::fs::write(&p, zip_of(&[("manifest.json", upd.as_bytes()), ("entities/b.json", b"{}")])).unwrap();
        install_archive(&d, "Mon monde", &p, true).unwrap();
        let bps: Vec<_> = list_packs(&d, "Mon monde").into_iter().filter(|p| p.kind == "behavior").collect();
        assert_eq!(bps.len(), 1);
        assert_eq!(bps[0].version, [2, 0, 0]);
        assert!(!d.join(BEHAVIOR_DIR).join(&bp.dir).join("entities/a.json").exists() || bps[0].dir == bp.dir);

        // Suppression : le pack disparaît aussi des mondes ; un pack du jeu est protégé.
        std::fs::create_dir_all(d.join("behavior_packs/vanilla")).unwrap();
        std::fs::write(d.join("behavior_packs/vanilla/manifest.json"), BP.replace("11111111-1111-4111-8111-111111111111", "55555555-5555-4555-8555-555555555555")).unwrap();
        assert!(list_packs(&d, "Mon monde").iter().any(|p| p.builtin));
        assert!(remove_pack(&d, "55555555-5555-4555-8555-555555555555").unwrap_err().to_string().contains("livré"));
        assert!(remove_pack(&d, "11111111-1111-4111-8111-111111111111").unwrap());
        assert!(!remove_pack(&d, "11111111-1111-4111-8111-111111111111").unwrap());
        let act: Vec<ActiveEntry> = serde_json::from_str(&std::fs::read_to_string(d.join("worlds/Mon monde/world_behavior_packs.json")).unwrap()).unwrap();
        assert!(act.is_empty());
        // Archives invalides.
        let bad = d.join("bad.mcpack");
        std::fs::write(&bad, zip_of(&[("readme.txt", b"x")])).unwrap();
        assert!(install_archive(&d, "Mon monde", &bad, true).unwrap_err().to_string().contains("aucun pack"));
        std::fs::write(&bad, b"pas un zip").unwrap();
        assert!(install_archive(&d, "Mon monde", &bad, true).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn nested_mcpack_inside_an_addon() {
        let d = tmp();
        let inner = zip_of(&[("manifest.json", RP.as_bytes())]);
        let outer = d.join("outer.mcaddon");
        std::fs::write(&outer, zip_of(&[("Textures.mcpack", &inner)])).unwrap();
        let ins = install_archive(&d, "w", &outer, false).unwrap();
        assert_eq!(ins.len(), 1);
        assert_eq!((ins[0].kind.as_str(), ins[0].active), ("resource", false));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn worlds_import_list_and_remove() {
        let d = tmp();
        let w = zip_of(&[("levelname.txt", "Ma Survie".as_bytes()), ("level.dat", b"LEVEL"), ("db/CURRENT", b"MANIFEST-1")]);
        let f = d.join("w.mcworld");
        std::fs::write(&f, &w).unwrap();
        assert_eq!(import_world(&d, &f, "autre", false).unwrap(), "Ma Survie");
        assert_eq!(std::fs::read(d.join("worlds/Ma Survie/db/CURRENT")).unwrap(), b"MANIFEST-1");
        assert!(import_world(&d, &f, "autre", false).unwrap_err().to_string().contains("existe déjà"));
        assert_eq!(import_world(&d, &f, "autre", true).unwrap(), "Ma Survie");
        // Un dossier racine unique et pas de levelname.txt : nom de repli.
        let f2 = d.join("w2.mcworld");
        std::fs::write(&f2, zip_of(&[("Dossier/level.dat", b"L"), ("Dossier/db/x", b"x")])).unwrap();
        assert_eq!(import_world(&d, &f2, "Importé", false).unwrap(), "Importé");
        // Nom de repli dangereux ou archive qui n'est pas un monde.
        assert!(import_world(&d, &f2, "../evil", false).is_err());
        let notw = d.join("n.mcworld");
        std::fs::write(&notw, zip_of(&[("a.txt", b"x")])).unwrap();
        assert!(import_world(&d, &notw, "x", false).unwrap_err().to_string().contains("pas un monde"));
        let l = list_worlds(&d, "Ma Survie");
        assert_eq!((l.len(), l[0].name.as_str(), l[0].active), (2, "Ma Survie", true), "monde actif en premier");
        assert!(remove_world(&d, "Ma Survie", "Ma Survie").is_err());
        assert!(remove_world(&d, "Importé", "Ma Survie").unwrap() && !remove_world(&d, "Importé", "Ma Survie").unwrap());
        assert!(remove_world(&d, "../x", "").is_err());
        assert!(valid_world_name("Mon monde") && !valid_world_name("a/b") && !valid_world_name("..") && !valid_world_name(".caché") && !valid_world_name(""));
        let _ = std::fs::remove_dir_all(&d);
    }
}
