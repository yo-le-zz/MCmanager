# MCManager sur macOS

Depuis la 1.1.1, chaque release contient un build **Mac Intel (x86_64)** :

| Fichier | Contenu |
|---|---|
| `mcmanager-<version>-macos-x86_64.dmg` | `MCManager.app` à glisser dans *Applications* |
| `mcmanager-<version>-macos-x86_64.tar.gz` | binaires `mcmanager` et `mcmanager-server` + dossier `web/` (usage en terminal) |

**Apple Silicon (M1 et suivants)** : aucun build natif n'est publié (les runners macOS Apple Silicon de GitHub sont trop souvent
indisponibles). Le build Intel fonctionne via **Rosetta 2**, installée d'une commande :
`softwareupdate --install-rosetta --agree-to-license`. Pour un binaire natif arm64, compilez sur place (`./build.sh macos`, voir plus bas).

Prérequis : **macOS 11 (Big Sur) ou plus récent**. Java n'est pas fourni (voir plus bas) ; sur un Mac Apple Silicon, choisissez
de préférence un Java natif arm64 (Homebrew `openjdk`), il est très supérieur à un Java Intel sous Rosetta.

## Installation
1. Ouvrez le `.dmg` et glissez **MCManager** dans *Applications*.
2. **Premier lancement** : l'application est signée « ad hoc » mais **pas notarisée** par Apple (cela demande un compte
   développeur payant). macOS affichera donc un avertissement. Au choix :
   - clic droit sur l'application → **Ouvrir** → **Ouvrir** ; ou
   - *Réglages Système → Confidentialité et sécurité → Ouvrir quand même* ; ou
   - dans un terminal : `xattr -dr com.apple.quarantine /Applications/MCManager.app`
3. Le navigateur s'ouvre sur `http://127.0.0.1:7777`. L'application n'a pas d'icône dans le Dock : pour la quitter, utilisez
   *Moniteur d'activité* → `mcmanager` → Quitter (ou `pkill mcmanager`). Relancer l'application alors qu'elle tourne rouvre simplement le navigateur.

En ligne de commande : `curl -fsSL https://mcmanager.pages.dev/install.sh | bash` installe l'archive portable dans `~/.local/share/mcmanager`.

## Java
MCManager lance vos serveurs avec Java (21 pour Minecraft ≥ 1.20.5, 17 pour 1.17–1.20.4, 8 jusqu'à 1.16).
Installation conseillée : `brew install openjdk@21` (ou @17), ou un JDK Temurin depuis adoptium.net.
Dans les réglages d'un serveur, le bouton **Détecter** trouve les Java installés (Homebrew, `/Library/Java/JavaVirtualMachines`,
JAVA_HOME, SDKMAN) et indique ceux qui conviennent à la version de Minecraft. L'application lancée depuis le Finder complète
elle-même le `PATH` avec les dossiers Homebrew.

## Données
`~/Library/Application Support/mcmanager` (variable `MCMANAGER_DATA_DIR` pour changer). Les serveurs Minecraft y sont créés
par défaut ; macOS peut demander l'autorisation d'accéder à *Documents* ou *Bureau* si vous placez un serveur dans ces dossiers.

## Mode serveur (`mcmanager-server`) avec launchd
Le service permanent automatique repose sur systemd (Linux). Sur un Mac qui sert de machine de jeu, créez
`~/Library/LaunchAgents/io.github.yo-le-zz.mcmanager-server.plist` :

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>io.github.yo-le-zz.mcmanager-server</string>
  <key>ProgramArguments</key><array>
    <string>/Users/VOUS/.local/share/mcmanager/mcmanager-server</string><string>--daemon</string>
  </array>
  <key>EnvironmentVariables</key><dict>
    <key>PATH</key><string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
    <key>MCMANAGER_ADMIN_USER</key><string>admin</string>
    <key>MCMANAGER_ADMIN_PASSWORD</key><string>changez-moi</string>
  </dict>
  <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
</dict></plist>
```
puis `launchctl load ~/Library/LaunchAgents/io.github.yo-le-zz.mcmanager-server.plist`.
Adaptez le chemin (`/Users/VOUS/...`) ; voir aussi [SERVER.md](SERVER.md).

## Compiler soi-même
Sur un Mac, avec [rustup](https://rustup.rs) : `./build.sh macos` produit `dist/MCManager.app`, le `.dmg` et le `.tar.gz`
pour l'architecture du Mac (pas de cross-compilation depuis Linux : le SDK Apple n'est pas redistribuable).

## Limites connues
- Pas de notarisation Apple (avertissement au premier lancement, voir plus haut).
- Pas de build natif Apple Silicon ni de binaire « universel » : Rosetta 2 est nécessaire sur M1 et suivants.
- La release macOS est produite par GitHub Actions ; elle n'a pas pu être essayée sur un vrai Mac avant la publication de la 1.1.1.
