# Changelog

Toutes les versions notables de MCManager sont documentées ici.
Format inspiré de [Keep a Changelog](https://keepachangelog.com/fr/1.0.0/).

## [1.0.1] - 2026-08-19
### Corrections
- **Paper ne proposait plus de choix de version** : PaperMC a retiré son
  ancienne API (`api.papermc.io/v2`) le 1er juillet 2026 au profit de la
  nouvelle API "Fill" (`fill.papermc.io/v3`). MCManager utilise maintenant
  la nouvelle API - Paper fonctionne à nouveau, avec en plus la possibilité
  de choisir un build precis (pas seulement la derniere version stable).
- **La console n'affichait rien tant qu'on ne changeait pas d'onglet** :
  ouvrir la console avant d'avoir jamais démarré le serveur connectait le
  websocket au mauvais canal interne. Corrigé - la console fonctionne dès
  le premier chargement.
- **La suppression d'un serveur ne supprimait pas son dossier** : les
  erreurs de suppression étaient silencieusement ignorées. Elles sont
  maintenant remontées à l'utilisateur, et les sauvegardes associées sont
  supprimées avec le serveur.
- **Sauvegardes non compatibles Windows** : les chemins internes du zip
  utilisaient le séparateur natif de l'OS (`\` sous Windows) au lieu de `/`
  comme l'exige le format zip.
- **`nix run github:yolezz/mcmanager` ne fonctionnait pas** : le flake.nix
  était dans `packaging/nix/` au lieu de la racine du dépôt. Déplacé à la
  racine - la commande standard fonctionne maintenant directement.
- Détection de timeout du ping serveur plus tolérante (un serveur qui vient
  de démarrer n'est plus signalé "hors ligne" par erreur).

### Ajouts
- Bouton pour vider la console.
- Import d'un serveur Minecraft déjà existant sur la machine.
- Choix d'un build/version de loader spécifique (Paper, Purpur, Fabric,
  Quilt, Forge) au lieu de toujours prendre la dernière version.
- Support de **Quilt** en plus de Fabric/Forge/NeoForge.
- Flags de performance JVM (Aikar) activables par serveur.
- Redémarrage automatique en cas de crash, avec distinction arrêt
  volontaire / crash.
- Confirmation avant d'envoyer une commande d'arrêt/redémarrage/reload,
  que ce soit via les boutons ou tapée directement dans la console.
- CLI headless (`mcmanager cli ...`) pour gérer une instance MCManager
  tournant sur un serveur distant sans navigateur.
- Utilisation d'une installation locale de `playit` déjà présente sur la
  machine, en plus du téléchargement automatique.
- Barre de progression pour la création de sauvegardes.
- Icône d'application dédiée, utilisée dans l'interface web, le favicon,
  et intégrée à l'exécutable Windows.
- Ouverture automatique du navigateur au lancement (désactivable).
- Plus d'options par serveur : arguments JVM additionnels, sauvegarde
  automatique, redémarrage automatique - éditables après création.

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

