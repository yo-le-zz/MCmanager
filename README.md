# MCManager

**Gestionnaire de serveurs Minecraft** — rapide, tout-en-un, avec interface web.
Réécriture complète en Rust (backend Axum) de l'ancienne version Python/Tkinter.

> Auteur : **yolezz** · Licence : **MIT** · Version : **1.0.5**

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
- **Statistiques par serveur** : CPU, RAM, joueurs en ligne (via ping du protocole Minecraft, sans plugin) — et un historique persistant (démarrages, temps de fonctionnement total, crashs) dans l'onglet Statistiques.
- **Console multi-ligne avec couleurs** : zone de saisie redimensionnable pour exécuter plusieurs commandes d'affilée, rendu des codes couleur ANSI/Minecraft, police et taille personnalisables.
- **Liste blanche** et **propriétés serveur** : onglets dédiés, pas besoin d'éditer les fichiers à la main.
- **Import/export de fichiers** en `.zip` (un dossier ou tout le serveur) depuis l'onglet Fichiers.
- **Notifications ntfy** (crash, sauvegarde, redémarrage programmé, arrêt automatique, connexion/déconnexion joueur) — pas de bot à configurer, juste un topic [ntfy.sh](https://ntfy.sh).
- **Assistant IA** intégré (Anthropic, OpenAI, Gemini, Ollama local, OmniRoute) avec chatbox de suggestions contextuelles.
- **playit.gg intégré** : téléchargement de l'agent, démarrage/arrêt, mini-tutoriel intégré pour exposer un serveur sur Internet sans configurer son routeur — ou utilisez directement une installation locale de `playit` déjà présente sur la machine.
- **Auto-mise à jour** : vérifie les tags GitHub au démarrage, propose et applique la mise à jour du binaire lui-même (comportement adapté selon `.deb`/Nix/portable — voir ci-dessous).
- **Redémarrage automatique en cas de crash**, redémarrage programmé, et arrêt automatique si le serveur reste vide — tous configurables et modifiables à tout moment, même après la création du serveur.
- **Diagnostic de crash automatique** : isole le mod/plugin fautif (ou une combinaison problématique) en testant les configurations une par une.
- **Import de serveurs existants** : pointez MCManager vers un dossier de serveur déjà présent sur la machine.
- **Deux binaires** : `mcmanager` (interface web) et **`mcmanager-headless`** (gestion 100% en ligne de commande, aucun serveur web, pensé pour un VPS sans navigateur — voir [docs/HEADLESS.md](docs/HEADLESS.md)). `mcmanager cli` reste disponible comme télécommande HTTP vers une instance `mcmanager` déjà lancée (voir [docs/CLI.md](docs/CLI.md)).
- **Ouverture automatique du navigateur** au lancement (désactivable via `MCMANAGER_NO_BROWSER=1`, utilisé automatiquement par les services systemd/Nix).
- **Multiplateforme** : Linux (.deb + script d'installation), NixOS/GLF OS (flake.nix à la racine + module NixOS — `nix run github:yo-le-zz/MCmanager`), Windows (portable + installateur Inno Setup).

### Mise à jour automatique selon le type d'installation
| Installation | Comportement |
|---|---|
| Portable (zip/tar.gz) | Mise à jour en un clic depuis l'UI |
| `.deb` | Désactivée volontairement (ne casse pas dpkg/apt) — l'appli indique comment mettre à jour via un nouveau `.deb` |
| Nix / `nix run` | Désactivée (store Nix en lecture seule) — relancez simplement `nix run github:yo-le-zz/MCmanager`, Nix récupère la dernière version automatiquement |


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
sudo dpkg -i mcmanager_1.0.1_amd64.deb
mcmanager
```
Ou via le script universel : `./install-linux.sh`.

### NixOS / GLF OS
```bash
nix run github:yo-le-zz/MCmanager
```
Ou en tant que service système, dans votre `flake.nix` :
```nix
inputs.mcmanager.url = "github:yo-le-zz/MCmanager";
# ...
services.mcmanager.enable = true;
```

### Windows
Décompressez `mcmanager-1.0.1-windows-x86_64.zip` et lancez `mcmanager.exe`,
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
- [docs/CLI.md](docs/CLI.md) — gérer MCManager en ligne de commande (serveurs distants)
- [docs/HEADLESS.md](docs/HEADLESS.md) — `mcmanager-headless`, binaire séparé sans aucun serveur web (VPS/Ubuntu)
- [docs/PLAYIT.md](docs/PLAYIT.md) — tutoriel détaillé playit.gg
- [docs/TUTO_INSTALLATION_GLFOS.md](docs/TUTO_INSTALLATION_GLFOS.md) — tuto d'installation simple pour NixOS/GLF OS
- L'onglet **Docs & tutos** de l'application contient aussi un guide intégré.

## 🧩 Architecture (pour contribuer)

```
src/
  lib.rs         racine de la bibliotheque partagee (tous les modules ci-dessous)
  main.rs        binaire "mcmanager" : point d'entrée web, routing HTTP, tâches de fond
  bin/headless.rs binaire "mcmanager-headless" : shell CLI sans serveur web (voir docs/HEADLESS.md)
  api.rs         toutes les routes REST
  ws.rs          WebSocket console + logs playit.gg
  state.rs       état applicatif partagé + persistance JSON + verrou d'instance
  models.rs      structures de données
  downloader.rs  téléchargement/installation des jars serveur
  modrinth.rs    client API Modrinth (marketplace)
  process.rs     cycle de vie des process serveur Java
  backup.rs      sauvegardes zip
  files.rs       navigateur/éditeur de fichiers, gestion mods/plugins
  playit.rs      intégration playit.gg
  presets.rs     préréglages "un clic" (dont le trousseau performance)
  debug.rs       diagnostic automatique de crash (test des addons un par un)
  stats.rs       CPU/RAM + ping Minecraft (joueurs en ligne)
  updater.rs     auto-mise à jour via GitHub releases
web/             interface web statique (HTML/CSS/JS, aucune étape de build)
packaging/       fichiers .deb, Nix, Windows (Inno Setup)
```

`mcmanager` (web) et `mcmanager-headless` (CLI pure, voir
[docs/HEADLESS.md](docs/HEADLESS.md)) sont deux binaires distincts qui
partagent tout leur code via la bibliothèque `src/lib.rs` — aucune logique
métier n'est dupliquée entre les deux.

Le code est volontairement modulaire pour être facilement étendu : chaque
fonctionnalité (marketplace, presets, playit...) vit dans son propre module et
peut être enrichie sans toucher au reste. Idées pour la suite : RCON natif,
tableaux de bord multi-machines, plugin système pour étendre l'API, thèmes UI,
support Bedrock (via geyser), etc. Les contributions sont bienvenues.

## 📄 Licence

MIT — voir [LICENSE](LICENSE).
