# MCManager v1.0.0 — Réécriture complète en Rust 🦀

MCManager passe d'une application de bureau Python/Tkinter à une **application
web légère en Rust**, pensée pour être simple pour un débutant complet tout en
restant puissante et extensible.

## ✨ Nouveautés principales

- 🔧 **Backend Rust (Axum)** + interface web — rapide, faible empreinte mémoire, utilisable en local ou à distance.
- 🧱 **Multi-loaders** : Vanilla, Paper, Purpur, Spigot, Fabric, Forge, NeoForge, avec **toutes les versions récupérées en direct** depuis les APIs officielles.
- 🪄 **Création de serveur en un clic** : un débutant choisit un type + une version, tout est téléchargé et configuré automatiquement (jar, EULA, server.properties).
- 🛒 **Marketplace intégré (Modrinth)** : recherche, installation et détection des mises à jour de mods/plugins directement dans l'interface, filtrée par loader et version.
- ⚡ **Préréglages "un clic"** : anti-cheat, essentiels, permissions, WorldEdit/FastAsyncWorldEdit, optimisation (Lithium, Chunky)...
- 🏗 **Support WorldEdit / FAWE** : dépôt de schematics directement depuis le navigateur.
- 💾 **Sauvegardes** : création/restauration/suppression en un clic, planification automatique.
- 📊 **Statistiques par serveur** : CPU, RAM, joueurs en ligne — sans plugin requis.
- 🌐 **playit.gg intégré** : téléchargement de l'agent, démarrage/arrêt, mini-tutoriel pas-à-pas pour jouer avec vos amis sans configurer votre routeur.
- 🔄 **Auto-mise à jour** de MCManager lui-même via les tags GitHub.
- 📦 **Empaquetage multiplateforme** : `.deb` pour Linux, flake Nix + module NixOS pour NixOS/GLF OS, build portable + installateur pour Windows.
- 📜 **Licence MIT**.

## 📥 Téléchargements

| Plateforme          | Fichier                                         |
|----------------------|--------------------------------------------------|
| Linux (Debian/Ubuntu) | `mcmanager_1.0.0_amd64.deb`                      |
| Linux (portable)     | `mcmanager-1.0.0-linux-x86_64.tar.gz`             |
| NixOS / GLF OS       | `mcmanager-1.0.0-nix.tar.gz` (flake.nix inclus)   |
| Windows (portable)   | `mcmanager-1.0.0-windows-x86_64.zip`              |
| Windows (installeur) | `mcmanager-1.0.0-setup.exe`                       |

## 🙏 Notes

Ce projet est pensé pour être **facilement amélioré par la communauté** :
architecture modulaire, API REST/WebSocket documentée, interface web sans
étape de build. Les retours et contributions sont les bienvenus !

**Changelog complet :** voir [CHANGELOG.md](./CHANGELOG.md)
