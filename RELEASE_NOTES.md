# MCManager v1.0.6 — Serveur dynamique, contrôle à distance chiffré RSA, site web

Trois gros ajouts cette fois : un mode "serveur dynamique" qui met un
serveur inactif en veille et le réveille à la demande, un système de
contrôle à distance chiffré pour `mcmanager-headless`, et le site
vitrine du projet.

## ⚡ Serveur dynamique (économie d'énergie)

Nouvelle option par serveur. Une fois qu'un serveur s'est arrêté
automatiquement par inactivité, un petit processus prend le relais sur son
port au lieu de le laisser mort :

- Il répond aux pings de la liste de serveurs Minecraft — le serveur
  reste visible, avec un message "💤 en veille" au lieu d'apparaître hors
  ligne.
- Dès qu'un joueur essaie *vraiment* de rejoindre, il démarre le vrai
  serveur et répond "réessayez dans quelques secondes" — plus simple et
  bien plus robuste qu'un relais TCP qui tenterait de faire patienter la
  connexion.
- Couvre le protocole **Java Edition** (testé de bout en bout sur de vrais
  sockets TCP) et, en **best-effort/expérimental**, **Bedrock via Geyser**.

**Compromis assumé** : la première connexion après une mise en veille
prend le temps de démarrage normal du serveur, pas moins. Des mods comme
[LazyDFU](https://modrinth.com/mod/lazydfu) ou
[Starlight](https://modrinth.com/plugin/starlight) peuvent réduire ce
délai — suggérés directement dans l'interface.

## 🔐 Contrôle à distance pour mcmanager-headless (RSA)

Option désactivée par défaut : `mcmanager-headless` peut exposer une API
de gestion sur le réseau pour être piloté depuis une autre machine.

- **Chiffrement hybride RSA-OAEP + AES-256-GCM** — même principe que
  TLS/SSH/PGP : RSA échange une clé de session, qui chiffre ensuite les
  échanges.
- **Chaque requête est signée** par la clé privée du client pour
  authentifier qui la fait.
- **Jumelage par code à usage unique** (10 minutes) affiché sur la
  machine hébergeant les serveurs — impossible de s'auto-jumeler en
  trouvant juste le port ouvert.
- Nouvelles commandes : `remote enable/pairing-code/clients/revoke` côté
  exposé, `remote pair/list/status/start/stop/restart/logs/send` côté
  pilote (qui peut être une autre installation de `mcmanager-headless`,
  y compris sur votre PC).

Le protocole est couvert par des tests unitaires et un **test
d'intégration qui fait tourner un vrai serveur HTTP et un vrai client
l'un contre l'autre** (jumelage, session, requête signée+chiffrée, et
rejet d'un client jamais jumelé) — pas seulement les fonctions de chiffrement
testées isolément.

## 🚀 Démarrage automatique, service systemd, menu Linux

- `autostart add <id>` relance un serveur à chaque lancement de
  `mcmanager-headless` (après un reboot, par exemple).
- Nouveau **service systemd système** `mcmanager-headless.service` :
  démarre au boot (pas seulement à la connexion d'un utilisateur) et
  redémarre automatiquement en cas d'échec.
- Le paquet `.deb` installe maintenant une **entrée dans le menu
  applications Linux** (visible à la touche Super sous GNOME/KDE), avec
  icône — le script d'installation portable (`install.sh`) fait de même.

## 🌐 Site web

Nouveau dossier `website/` (à déployer sur mcmanager.pages.dev), basé sur
une maquette Stitch adaptée avec des liens et commandes réels :
téléchargement Windows, `curl | bash` pour Linux, `nix run` pour NixOS —
toutes résolvent automatiquement la dernière release GitHub. Les scripts
d'installation sont aussi disponibles dans `/scripts` à la racine du
dépôt.

## 📥 Téléchargements

| Plateforme | Fichier |
|---|---|
| Linux (Debian/Ubuntu) | `mcmanager_1.0.6_amd64.deb` (inclut `mcmanager`, `mcmanager-headless`, service systemd, menu applications) |
| Linux (portable) | `mcmanager-1.0.6-linux-x86_64.tar.gz` |
| NixOS / GLF OS | `nix run github:yo-le-zz/MCmanager` |
| Windows (portable) | `mcmanager-1.0.6-windows-x86_64.zip` |
| Windows (installeur) | `mcmanager-1.0.6-setup.exe` |

**Changelog complet :** voir [CHANGELOG.md](./CHANGELOG.md)

## ⏭ Pas encore fait

- RCON / suivi TPS en direct.
- Le mode Bedrock/Geyser du serveur dynamique reste expérimental — non
  testé contre un vrai client Bedrock (RakNet implémenté au niveau
  protocole, mais pas exercé en conditions réelles).
- Couverture i18n toujours partielle.
