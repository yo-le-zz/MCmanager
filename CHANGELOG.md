# Changelog

Toutes les versions notables de MCManager sont documentées ici.
Format inspiré de [Keep a Changelog](https://keepachangelog.com/fr/1.0.0/).

## [1.0.0] - 2026-08-18
### Réécriture complète en Rust
- Remplacement total de l'ancienne application Python/Tkinter par un backend
  Rust (Axum) + interface web, utilisable en local ou à distance.
- Support multi-loaders : Vanilla, Paper, Purpur, Spigot, Fabric, Forge, NeoForge.
- Récupération dynamique des versions disponibles (aucune version codée en dur).
- Marketplace intégré (API Modrinth) avec recherche, installation et détection
  de mises à jour par empreinte de fichier.
- Préréglages "un clic" (anti-cheat, essentiels, permissions, WorldEdit/FAWE...).
- Support des schematics WorldEdit / FastAsyncWorldEdit.
- Sauvegardes zip avec restauration et planification automatique.
- Statistiques par serveur (CPU, RAM, joueurs en ligne) sans plugin requis.
- Intégration playit.gg (téléchargement, démarrage, tutoriel intégré).
- Auto-mise à jour de l'application via les releases GitHub.
- Empaquetage : `.deb` (Linux), Nix flake + module NixOS (GLF OS...),
  build Windows portable + installateur Inno Setup.
- Licence changée pour MIT.

## [Historique pré-Rust]
- 4.0.0 et versions antérieures : application Python (customtkinter), voir
  l'historique Git pour le détail — non maintenue depuis la réécriture 1.0.0.
