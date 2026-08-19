use crate::models::PresetItem;

/// Curated defaults a beginner can install in one click. Slugs are Modrinth
/// project slugs; `search`/`install` re-resolve the right build for the
/// server's loader + Minecraft version at install time.
pub fn all() -> Vec<PresetItem> {
    vec![
        PresetItem {
            key: "anticheat-grim".into(),
            label: "GrimAC (Anti-Cheat)".into(),
            description: "Detection de triche (kill aura, fly, speed...) cote serveur.".into(),
            category: "Anti-Cheat".into(),
            modrinth_slug: "grimac".into(),
            loaders: vec!["paper".into(), "purpur".into(), "spigot".into()],
        },
        PresetItem {
            key: "essentials".into(),
            label: "EssentialsX".into(),
            description: "Commandes de base : /tp, /home, /kit, economie, warps...".into(),
            category: "Essentiels".into(),
            modrinth_slug: "essentialsx".into(),
            loaders: vec!["paper".into(), "purpur".into(), "spigot".into()],
        },
        PresetItem {
            key: "worldedit".into(),
            label: "WorldEdit".into(),
            description: "Edition de terrain massive, import/export de schematics.".into(),
            category: "Construction".into(),
            modrinth_slug: "worldedit".into(),
            loaders: vec!["paper".into(), "purpur".into(), "spigot".into(), "fabric".into(), "quilt".into(), "forge".into()],
        },
        PresetItem {
            key: "fawe".into(),
            label: "FastAsyncWorldEdit (FAWE)".into(),
            description: "Version optimisee de WorldEdit, tres rapide sur gros volumes.".into(),
            category: "Construction".into(),
            modrinth_slug: "fastasyncworldedit".into(),
            loaders: vec!["paper".into(), "purpur".into()],
        },
        PresetItem {
            key: "viaversion".into(),
            label: "ViaVersion".into(),
            description: "Permet aux clients de versions plus recentes de se connecter.".into(),
            category: "Compatibilite".into(),
            modrinth_slug: "viaversion".into(),
            loaders: vec!["paper".into(), "purpur".into(), "spigot".into()],
        },
        PresetItem {
            key: "luckperms".into(),
            label: "LuckPerms".into(),
            description: "Gestion fine des permissions et des grades.".into(),
            category: "Administration".into(),
            modrinth_slug: "luckperms".into(),
            loaders: vec!["paper".into(), "purpur".into(), "spigot".into(), "fabric".into(), "quilt".into(), "forge".into()],
        },
        PresetItem {
            key: "chunky".into(),
            label: "Chunky".into(),
            description: "Pre-generation efficace des chunks pour eviter le lag a l'exploration.".into(),
            category: "Performance".into(),
            modrinth_slug: "chunky".into(),
            loaders: vec!["paper".into(), "purpur".into(), "spigot".into(), "fabric".into(), "quilt".into(), "forge".into()],
        },
        PresetItem {
            key: "lithium".into(),
            label: "Lithium".into(),
            description: "Optimisations serveur sans changement de gameplay (Fabric/Forge).".into(),
            category: "Performance".into(),
            modrinth_slug: "lithium".into(),
            loaders: vec!["fabric".into(), "quilt".into(), "forge".into(), "neoforge".into()],
        },
    ]
}
