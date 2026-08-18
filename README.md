# MCManager

**Gestionnaire de serveurs Minecraft** — rapide, tout-en-un, avec interface web.
Réécriture complète en Rust (backend Axum) de l'ancienne version Python/Tkinter.

> Auteur : **yolezz** · Licence : **MIT** · Version : **1.0.0**

---

## ✨ Fonctionnalités

- **Multi-serveurs** : Vanilla, Paper, Purpur, Spigot, Fabric, Forge, NeoForge — toutes versions disponibles récupérées en direct depuis les APIs officielles (Mojang, PaperMC, Purpur, Fabric, Forge, NeoForge), pas de liste figée.
- **Création en un clic** : choisissez un type de serveur + une version, MCManager télécharge le bon jar (ou lance l'installeur Forge/NeoForge), crée `eula.txt`, `server.properties`, etc. Pensé pour un débutant complet.
- **Console web en temps réel** (WebSocket) avec envoi de commandes.
- **Marketplace intégré (Modrinth)** : recherche de mods/plugins filtrée automatiquement par loader + version, installation en un clic, détection des mises à jour disponibles.
- **Préréglages "par défaut"** : anti-cheat (GrimAC), EssentialsX, LuckPerms, ViaVersion, WorldEdit/FastAsyncWorldEdit, Chunky, Lithium — un clic pour poser les bases d'un serveur.
- **WorldEdit / FastAsyncWorldEdit** : dépôt de fichiers `.schem` / `.schematic` directement depuis l'interface web.
- **Gestion des mods/plugins installés** : activer / désactiver / supprimer.
- **Éditeur de fichiers** intégré (server.properties, configs de plugins/mods…) avec navigateur de fichiers sécurisé (anti path-traversal).
- **Sauvegardes** : création/restauration/suppression en `.zip`, sauvegarde automatique programmable par serveur.
- **Statistiques par serveur** : CPU, RAM, joueurs en ligne (via ping du protocole Minecraft, sans plugin).
- **playit.gg intégré** : téléchargement de l'agent, démarrage/arrêt, mini-tutoriel intégré pour exposer un serveur sur Internet sans configurer son routeur.
- **Auto-mise à jour** : vérifie les tags GitHub au démarrage, propose et applique la mise à jour du binaire lui-même.
- **Multiplateforme** : Linux (.deb + script d'installation), NixOS/GLF OS (flake.nix + module NixOS), Windows (portable + installateur Inno Setup).

---

## 🚀 Démarrage rapide

```bash
# Lancer directement le binaire compilé
./mcmanager
# Interface web disponible sur http://127.0.0.1:7777
```

Variables d'environnement utiles :

| Variable              | Défaut          | Description                                   |
|-----------------------|-----------------|------------------------------------------------|
| `MCMANAGER_HOST`      | `127.0.0.1`     | Adresse d'écoute du serveur web                |
| `MCMANAGER_PORT`      | `7777`          | Port de l'interface web                        |
| `MCMANAGER_DATA_DIR`  | dossier système | Où sont stockés serveurs, backups, config      |
| `MCMANAGER_WEB_DIR`   | auto-détecté    | Dossier contenant `index.html`/`app.js`/`style.css` |

## 🖥 Installation

### Linux (Debian/Ubuntu)
```bash
sudo dpkg -i mcmanager_1.0.0_amd64.deb
mcmanager
```
Ou via le script universel : `./install-linux.sh`.

### NixOS / GLF OS
```bash
nix run github:yolezz/mcmanager
```
Ou en tant que service système, dans votre `flake.nix` :
```nix
inputs.mcmanager.url = "github:yolezz/mcmanager";
# ...
services.mcmanager.enable = true;
```

### Windows
Décompressez `mcmanager-1.0.0-windows-x86_64.zip` et lancez `mcmanager.exe`,
ou utilisez l'installateur généré via `dist/mcmanager.iss` (Inno Setup).

---

## 🏗 Compiler soi-même

```bash
cargo build --release
./build.sh          # génère tous les paquets dans ./dist
./build.sh linux     # uniquement Linux (.deb + tar.gz)
./build.sh windows   # cross-compilation Windows (nécessite mingw-w64)
./build.sh nix       # empaquetage Nix
./build.sh installer # scripts d'installation / Inno Setup
```

Voir [docs/BUILD.md](docs/BUILD.md) pour le détail des prérequis par plateforme.

---

## 📚 Documentation

- [docs/BUILD.md](docs/BUILD.md) — compiler et empaqueter pour chaque plateforme
- [docs/API.md](docs/API.md) — toutes les routes de l'API REST/WebSocket
- [docs/PLAYIT.md](docs/PLAYIT.md) — tutoriel détaillé playit.gg
- L'onglet **Docs & tutos** de l'application contient aussi un guide intégré.

## 🧩 Architecture (pour contribuer)

```
src/
  main.rs        point d'entrée, routing HTTP, tâches de fond
  api.rs         toutes les routes REST
  ws.rs          WebSocket console + logs playit.gg
  state.rs       état applicatif partagé + persistance JSON
  models.rs      structures de données
  downloader.rs  téléchargement/installation des jars serveur
  modrinth.rs    client API Modrinth (marketplace)
  process.rs     cycle de vie des process serveur Java
  backup.rs      sauvegardes zip
  files.rs       navigateur/éditeur de fichiers, gestion mods/plugins
  playit.rs      intégration playit.gg
  presets.rs     préréglages "un clic"
  stats.rs       CPU/RAM + ping Minecraft (joueurs en ligne)
  updater.rs     auto-mise à jour via GitHub releases
web/             interface web statique (HTML/CSS/JS, aucune étape de build)
packaging/       fichiers .deb, Nix, Windows (Inno Setup)
```

Le code est volontairement modulaire pour être facilement étendu : chaque
fonctionnalité (marketplace, presets, playit...) vit dans son propre module et
peut être enrichie sans toucher au reste. Idées pour la suite : RCON natif,
tableaux de bord multi-machines, plugin système pour étendre l'API, thèmes UI,
support Bedrock (via geyser), etc. Les contributions sont bienvenues.

## 📄 Licence

MIT — voir [LICENSE](LICENSE).
