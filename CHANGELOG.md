# Changelog

Toutes les versions notables de MCManager sont documentées ici.
Format inspiré de [Keep a Changelog](https://keepachangelog.com/fr/1.0.0/).

## [1.0.2] - 2026-08-20 (i18n, UI/UX, mods/plugins avances, mode debug, CLI, assistant IA)
### Ajouts
- **Support multilingue (FR / EN / ES)** : nouvelle infrastructure `web/i18n.js`
  avec sélecteur de langue dans Paramètres, détection automatique de la
  langue du navigateur, et repli sur le français pour tout ce qui n'est pas
  encore traduit. Couvre le menu, les boutons communs, les paramètres, et
  la page Docs & tutoriels dans son intégralité.
- **Bouton "Ouvrir dans l'explorateur"** : sur chaque serveur (liste
  Serveurs) et dans la barre d'outils Fichiers, ouvre le dossier
  correspondant dans l'explorateur natif de l'OS (`explorer`/`open`/
  `xdg-open` selon la plateforme). Nouvelle route
  `POST /api/servers/:id/open-folder`.
- **Bouton "Config" par mod/plugin** : dans l'onglet Mods/Plugins, ouvre
  directement le dossier `mods/`/`plugins/` du serveur dans l'onglet
  Fichiers pour éditer sa configuration.
- **Paramètres serveur étendus** : délai de redémarrage après crash
  configurable (remplace le délai fixe de 5s), redémarrage programmé
  (toutes les N minutes), arrêt automatique si aucun joueur n'a rejoint
  depuis N minutes — modifiables à tout moment depuis Paramètres, même
  après la création du serveur, sans attendre un redémarrage manuel.
- **Bouton "Ajouter les mods/plugins de performance"** dans l'onglet
  Mods/Plugins : installe en un clic un trousseau curé (Chunky, Lithium,
  spark...) compatible avec le loader du serveur.
- **Mods/plugins "gérés"** : liste définie par l'utilisateur (par ID/slug
  Modrinth) que MCManager garde dans la bonne version pour ce serveur ;
  rien ne se télécharge automatiquement, un bouton "Synchroniser
  maintenant" déclenche la mise à jour quand l'utilisateur le décide.
  Ajoutable aussi directement depuis les résultats du Marketplace
  ("➕ Suivi auto").
- **Mode debug (diagnostic de crash)** : nouveau bouton dans la Console
  qui teste d'abord la configuration actuelle telle quelle (rien à
  toucher si elle démarre déjà), puis, si besoin, désactive tous les
  mods/plugins, vérifie que le serveur démarre nu, et les réactive un par
  un (en isolant chacun) pour trouver lequel provoque un crash à lui
  seul. Si aucun addon ne plante seul mais que l'ensemble complet ne
  démarre pas, une phase supplémentaire réactive les addons un par un de
  façon cumulative pour repérer une combinaison problématique entre
  plusieurs d'entre eux. Tout est remis dans l'état d'origine à la fin,
  et la progression s'affiche en direct dans la console.
- **Binaire CLI séparé `mcmanager-headless`** : gère les serveurs
  (création, démarrage/arrêt, mods/plugins, diagnostic...) entièrement en
  ligne de commande, sans jamais démarrer de serveur web/API HTTP — pensé
  pour un VPS/serveur Ubuntu sans navigateur. Partage le même code et le
  même format de données que `mcmanager` (web) via une nouvelle
  bibliothèque partagée (`src/lib.rs`). Un verrou d'instance
  (`mcmanager.lock`) empêche de lancer les deux binaires en même temps sur
  le même dossier de données. Voir `docs/HEADLESS.md`.
- **Assistant IA** (nouvel onglet "Assistant IA") : chatbox qui donne des
  suggestions sur quoi ajouter/modifier/réparer, avec le contexte du
  serveur sélectionné (loader, version, mods/plugins installés, état).
  Quatre fournisseurs au choix : Anthropic (Claude), OpenAI, Google Gemini,
  ou Ollama en local. Le fournisseur est détecté automatiquement à partir
  du format de la clé collée, et la liste des modèles disponibles est
  récupérée directement auprès du fournisseur. La clé API est chiffrée
  sur disque (AES-256-GCM) avec une clé de chiffrement générée localement
  et stockée séparément (`ai_key.bin`, accès restreint au propriétaire du
  compte) — une vraie protection contre une copie/sauvegarde accidentelle
  du seul fichier de config, mais pas l'équivalent d'un trousseau système
  puisque la clé de déchiffrement reste sur la même machine (indiqué
  clairement dans l'interface). Pour Ollama en local uniquement,
  l'assistant dispose d'outils de recherche web et de lecture de page
  pour compenser l'absence de connaissances à jour d'un modèle local :
  extraction des résultats DuckDuckGo par bloc (titre + extrait + URL),
  avec repli automatique sur l'API OpenSearch de Wikipedia (stable, sans
  clé) si DuckDuckGo échoue ou ne répond pas.
### Changements
- **Mise en page pleine hauteur** : la console (jeu et playit.gg) et
  l'explorateur/éditeur de fichiers occupent maintenant tout l'espace
  vertical disponible au lieu d'une hauteur fixe (460px/500px), sur tous
  les formats d'écran.
- **Suppression des préréglages recommandés** de l'onglet Mods/Plugins
  (prenaient de la place sans utilité avérée). L'API `/api/presets` reste
  disponible côté backend si besoin futur.
### Corrections
- **playit.gg n'affichait rien au démarrage tant que le binaire était déjà
  téléchargé** : la sortie de l'agent imprimée entre la fin de la requête
  `POST /playit/start` et l'ouverture du WebSocket de la console était
  perdue (aucune association possible avant coup). Un tampon de rediffusion
  (`playit_backlog`, même principe que la console des serveurs) a été
  ajouté : les dernières lignes sont maintenant rejouées à la connexion.

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

