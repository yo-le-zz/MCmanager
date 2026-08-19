# MCManager v1.0.1 — Corrections importantes + nouvelles fonctionnalités

Cette version corrige plusieurs bugs bloquants remontés par les premiers
utilisateurs (dont un vrai changement d'API côté PaperMC) et ajoute une
bonne dizaine de fonctionnalités demandées.

## 🐛 Corrections importantes

- **Paper ne proposait aucune version au choix** : PaperMC a coupé son
  ancienne API le 1er juillet 2026. MCManager utilise maintenant la
  nouvelle API "Fill" officielle - Paper fonctionne à nouveau, avec en
  bonus la possibilité de choisir un build précis.
- **Console vide au premier chargement** : il fallait changer d'onglet puis
  revenir pour voir apparaître la sortie du serveur. Corrigé.
- **Suppression de serveur incomplète** : le dossier du serveur (et ses
  sauvegardes) n'étaient pas toujours supprimés, sans avertissement. Les
  erreurs sont maintenant visibles, et les sauvegardes sont nettoyées aussi.
- **Sauvegardes cassées entre Linux et Windows** : chemins internes du zip
  non conformes au format sous Windows.
- **`nix run github:yolezz/mcmanager` ne fonctionnait pas** : le flake était
  mal placé dans le dépôt. C'est corrigé - la commande standard fonctionne
  directement, plus besoin de récupérer le projet en entier.

## ✨ Nouveautés

- Bouton pour vider la console, avec confirmation avant tout arrêt/reload
  (bouton ou commande tapée directement).
- Import d'un serveur Minecraft déjà existant sur la machine.
- Choix d'un build/version de loader précis (Paper, Purpur, Fabric, Quilt, Forge).
- Support de **Quilt**.
- Flags de performance JVM (Aikar) activables par serveur.
- Redémarrage automatique en cas de crash.
- CLI headless (`mcmanager cli list/status/start/stop/create`) pour gérer
  MCManager sur un serveur distant sans navigateur.
- Utilisation d'une installation locale de `playit` en plus du téléchargement auto.
- Barre de progression pour les sauvegardes.
- Icône d'application dédiée (UI, favicon, exécutable Windows).
- Ouverture automatique du navigateur au lancement.
- Plus d'options éditables par serveur (arguments JVM, sauvegarde auto, auto-restart).

## 📥 Téléchargements

| Plateforme | Fichier |
|---|---|
| Linux (Debian/Ubuntu) | `mcmanager_1.0.1_amd64.deb` |
| Linux (portable) | `mcmanager-1.0.1-linux-x86_64.tar.gz` |
| NixOS / GLF OS | `nix run github:yolezz/mcmanager` (plus besoin de télécharger quoi que ce soit) |
| Windows (portable) | `mcmanager-1.0.1-windows-x86_64.zip` |
| Windows (installeur) | `mcmanager-1.0.1-setup.exe` |

**Changelog complet :** voir [CHANGELOG.md](./CHANGELOG.md)
