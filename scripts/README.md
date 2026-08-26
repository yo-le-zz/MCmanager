# scripts/

Scripts d'installation MCManager (identiques à ceux servis par
`website/install.sh` et `website/install.ps1` sur mcmanager.pages.dev).

## install.sh (Linux, x86_64)

```bash
curl -fsSL https://mcmanager.pages.dev/install.sh | bash
```

- Installe via le paquet `.deb` si `dpkg` est disponible (intégration la
  plus propre : service systemd, désinstallation via `apt`/la logithèque).
- Sinon, installe l'archive portable dans `~/.local/share/mcmanager`, avec
  un lien dans `~/.local/bin` et une entrée dans le menu applications
  (visible à la touche Super sous GNOME/KDE).
- Désinstaller l'installation portable :
  ```bash
  curl -fsSL https://mcmanager.pages.dev/install.sh | bash -s -- --uninstall
  ```
  (pour le `.deb`, utiliser `sudo apt remove mcmanager` comme pour tout paquet).

## install.ps1 (Windows)

```powershell
iex (irm mcmanager.pages.dev/install.ps1)
```

Télécharge et lance l'installateur `.exe` (Inno Setup) de la dernière
release GitHub — c'est cet installateur qui crée le raccourci dans le menu
Démarrer et enregistre la désinstallation dans "Applications installées"
(Windows le gère nativement, rien à recréer à la main).

- Installation silencieuse : `iex "& { $(irm mcmanager.pages.dev/install.ps1) } -Silent"`
- Désinstaller : `iex "& { $(irm mcmanager.pages.dev/install.ps1) } -Uninstall"`
  (ou Paramètres Windows → Applications installées → MCManager → Désinstaller)

## NixOS / GLF OS

Pas de script séparé — la commande sur le site (`nix run
github:yo-le-zz/MCmanager`) utilise directement le flake du dépôt
(`flake.nix` à la racine), qui expose aussi un module NixOS
(`services.mcmanager.enable = true;`).
