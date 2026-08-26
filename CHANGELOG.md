# Changelog

Toutes les versions notables de MCManager sont documentées ici.
Format inspiré de [Keep a Changelog](https://keepachangelog.com/fr/1.0.0/).

## [1.0.6] - 2026-08-24 (serveur dynamique, controle a distance RSA, site web)
### Ajouts
- **"Serveur dynamique"** (option par serveur, économie d'énergie) : une
  fois qu'un serveur s'est arrêté automatiquement par inactivité
  (`stop_when_empty_minutes`), un petit processus prend le relais sur son
  port au lieu de le laisser mort. Il répond aux pings de la liste de
  serveurs (le serveur reste visible avec un message "en veille") et, dès
  qu'un joueur essaie vraiment de rejoindre, démarre le vrai serveur et
  répond poliment "réessayez dans quelques secondes" plutôt que de tenter
  un relais TCP fragile. Couvre le protocole Java Edition (implémentation
  testée de bout en bout, y compris sur un vrai socket TCP) et, en
  best-effort/expérimental, Bedrock via Geyser (ping RakNet + détection de
  tentative de connexion réelle). Compromis assumé et expliqué dans
  l'interface : la première connexion après une mise en veille prend le
  temps de démarrage normal du serveur - des mods comme LazyDFU ou
  Starlight peuvent réduire ce délai.
- **Contrôle à distance pour `mcmanager-headless`**, chiffré et authentifié
  par RSA (option, désactivée par défaut) : expose une API de gestion sur
  le réseau (`0.0.0.0`) pour piloter des serveurs depuis une autre
  machine. Chiffrement hybride RSA-OAEP + AES-256-GCM (même principe que
  TLS/SSH/PGP - RSA échange une clé de session, qui chiffre ensuite les
  échanges), chaque requête signée par la clé privée du client pour
  authentifier qui la fait, jumelage par code à usage unique (10 minutes)
  affiché sur la machine hébergeant les serveurs - impossible de
  s'auto-jumeler juste en trouvant le port ouvert. Nouvelles commandes
  `remote enable/disable/pairing-code/clients/revoke` (côté exposé) et
  `remote pair/targets/list/status/start/stop/restart/logs/send` (côté
  pilote - peut être une autre install de `mcmanager-headless`). Le
  protocole est couvert par des tests unitaires et un test d'intégration
  qui fait tourner un vrai serveur HTTP et un vrai client l'un contre
  l'autre (jumelage, session, requête signée+chiffrée, rejet d'un client
  non jumelé).
- **Démarrage automatique de serveurs** (`autostart add/remove/list <id>`)
  et **mode daemon** (`--daemon`, attend indéfiniment au lieu de quitter
  immédiatement sans terminal attaché) pour `mcmanager-headless`.
- **Service systemd système `mcmanager-headless.service`** (paquet .deb) :
  démarre au boot (pas seulement à la connexion d'un utilisateur, contrairement
  au service web existant) et redémarre automatiquement en cas d'échec
  (`Restart=on-failure`).
- **Menu applications Linux** : le paquet `.deb` installe maintenant une
  entrée `.desktop` + icône (visible à la touche Super sous GNOME/KDE) ;
  le script d'installation portable (`install.sh`) fait de même.
- **Site web** (`website/`, à déployer sur mcmanager.pages.dev) : page de
  présentation, commandes d'installation Windows/Linux/NixOS fonctionnelles
  (résolvent toujours la dernière release GitHub), copié depuis une
  maquette Stitch adaptée avec des liens réels. Scripts d'installation
  (`install.sh`, `install.ps1`) dupliqués dans `/scripts` à la racine du
  dépôt.

## [1.0.5] - 2026-08-23 (correctifs Java critiques, IA agissante, markdown, suivi de plugins)
### Corrections critiques
- **Le chemin Java configuré n'était jamais utilisé pour un serveur déjà
  créé.** Il n'existait qu'un réglage Java **global**, appliqué uniquement
  aux serveurs créés *après* l'avoir changé - un serveur existant restait
  bloqué sur le Java par défaut du système. C'est ce qui causait des
  erreurs comme "Invalid maximum heap size" avec `-Xmx4096M` alors que la
  même commande fonctionnait très bien lancée à la main avec le bon Java.
  Ajout d'un champ **Java par serveur** dans Paramètres, avec un bouton
  **"🧪 Tester"** qui lance réellement `<ce java> -Xmx<valeur> -version`
  et affiche le resultat exact - pour detecter le probleme avant de
  démarrer le serveur, pas après un crash.
- **Les paramètres de serveur ne semblaient pas s'enregistrer.** Après un
  clic sur "Enregistrer", l'interface republiait les anciennes valeurs
  parce que le cache local des serveurs n'était mis à jour qu'au
  changement de page, jamais juste après la sauvegarde - alors que le
  serveur avait bien enregistré le changement côté backend. Corrigé : la
  réponse de sauvegarde met maintenant directement à jour ce cache.
### Ajouts
- **Markdown dans les réponses de l'assistant IA** : gras, listes, blocs
  de code, liens, titres sont maintenant rendus proprement au lieu de
  s'afficher en texte brut avec des astérisques. Rendu maison, aucune
  dépendance externe.
- **L'assistant IA peut agir, pas seulement suggérer** : nouvel outil
  "installer un mod/plugin" branché sur Anthropic et Ollama (boucle
  d'appel d'outils bornée à 4 étapes pour éviter qu'un modèle confus ne
  boucle indéfiniment), plus un outil pour lister ce qui est déjà
  installé avant de proposer un doublon.
- **Suivre un mod/plugin déjà installé** : nouveau bouton "👁 Suivre" sur
  chaque addon de l'onglet Mods/Plugins, qui l'identifie via Modrinth (par
  empreinte de fichier, comme la vérification de mises à jour) et l'ajoute
  à la liste "gérée" sans avoir à connaître son slug/ID Modrinth.
- **Traductions manquantes comblées** : les titres de toutes les pages
  (Tableau de bord, Serveurs, Console, Fichiers, Marketplace, Sauvegardes,
  Statistiques, Liste blanche, Propriétés serveur, Réseau, Assistant IA,
  Paramètres) et plusieurs boutons "Enregistrer"/"Supprimer" restants
  passent maintenant par `web/i18n.js` au lieu d'être figés en français.
  Couverture toujours partielle (voir ci-dessous) - reste principalement
  le contenu long des cartes de paramètres et certains labels de formulaire.

## [1.0.4] - 2026-08-23 (notifications, statistiques, liste blanche, propriétés serveur, console améliorée, import/export)
### Ajouts
- **Notifications ntfy** (au lieu d'un bot Discord — plus simple, pas de compte
  d'application à créer) : alertes crash, sauvegarde terminée, redémarrage
  programmé, arrêt automatique, connexion/déconnexion joueur. Un topic
  suffit (public [ntfy.sh](https://ntfy.sh) ou instance auto-hébergée), avec
  jeton d'authentification optionnel chiffré sur disque. Bascule par type
  d'événement et bouton "Envoyer un test" dans Paramètres.
- **Onglet Statistiques** : nombre de démarrages, temps de fonctionnement
  total, nombre de crashs détectés, et journal des sessions récentes (début,
  fin, durée, propre ou crash) par serveur.
- **Onglet Liste blanche** : ajouter/retirer des joueurs (via commande
  console si le serveur tourne, ou édition directe de `whitelist.json` à
  l'arrêt), activer/désactiver l'application de la liste blanche.
- **Onglet Propriétés serveur** : formulaire convivial pour les réglages
  courants de `server.properties` (difficulté, mode de jeu, PvP,
  max-players, distance de vue, mode en ligne...) avec les bons types de
  contrôle (case à cocher, liste déroulante, nombre) au lieu de texte brut.
  Un éditeur "fichier complet" reste disponible pour les clés non listées.
- **Console repensée** : zone de saisie multi-ligne et redimensionnable
  (une commande par ligne, bouton "▶ Exécuter" ou Ctrl+Entrée pour tout
  envoyer d'affilée) — fini le champ trop étroit pour une commande `give`
  avec NBT custom. **Couleurs dans le terminal** : les codes ANSI
  (Paper/Spigot) et les codes couleur Minecraft (`§`) sont maintenant
  rendus en couleur au lieu de s'afficher en brut. **Apparence
  personnalisable** (taille de police, police) dans Paramètres, appliquée
  à toutes les consoles.
- **Import/export de fichiers** dans l'onglet Fichiers : export d'un
  dossier (ou de tout le serveur) en `.zip` téléchargeable, import d'une
  archive `.zip` avec extraction protégée contre le "zip-slip"
  (chemins `../` malveillants dans l'archive rejetés un par un).
- **Rétention des sauvegardes** : réglage optionnel par serveur ("garder
  les N dernières"), appliqué automatiquement après chaque nouvelle
  sauvegarde (manuelle ou automatique).
- **Animations d'interface** : transitions sur la navigation, les cartes,
  les boutons (retour visuel au clic) et les notifications toast, léger
  fondu à l'affichage de chaque page.
- **Fournisseur OmniRoute** pour l'assistant IA
  ([omniroute.online](https://omniroute.online/),
  [github.com/diegosouzapw/OmniRoute](https://github.com/diegosouzapw/OmniRoute)) :
  passerelle auto-hébergée compatible OpenAI donnant accès à 300+
  fournisseurs derrière une seule clé, détection des modèles disponibles,
  modèle par défaut `auto`.
### Changements
- **`.lock` d'instance plus intelligent** : au lieu de refuser
  systématiquement de démarrer si un fichier de verrou existe, MCManager
  vérifie maintenant (via `sysinfo`) si le PID qu'il référence appartient
  toujours à un processus vivant, et si c'est bien MCManager. Un verrou
  laissé par un arrêt brutal (processus mort) ou par un PID réutilisé par
  un autre programme est désormais proposé à la suppression de façon
  interactive, avec repli sûr (refus) en contexte non-interactif
  (service systemd, stdin fermé) plutôt que de bloquer indéfiniment ou de
  supprimer un verrou potentiellement encore valide.
- Clé de chiffrement locale mutualisée (`secrets.rs`) entre l'assistant IA
  et les notifications ntfy, avec migration automatique de l'ancienne clé
  pour ne pas invalider les configurations IA déjà enregistrées.
### Corrections
- **L'installateur Windows (`mcmanager-*-setup.exe`) n'était jamais généré
  ni inclus dans les releases.** Le workflow de CI le compilait sur le job
  **Linux**, où Inno Setup n'est jamais disponible : l'étape échouait
  silencieusement. Déplacée sur le job Windows, avec installation d'Inno
  Setup via Chocolatey.
- **La mise à jour automatique ne détectait jamais rien.** Cause racine :
  le code (vérification de mise à jour, user-agent HTTP, liens `nix run`,
  README, documentation, scripts de build) pointait vers
  `github.com/yolezz/mcmanager`, un dépôt qui n'existe pas — le vrai dépôt
  est `yo-le-zz/MCmanager`. Chaque vérification échouait silencieusement
  (404), donc aucune mise à jour n'était jamais détectée, quel que soit le
  tag publié. Corrigé partout.
- **La création de release GitHub échouait avec une erreur 403** (le
  `GITHUB_TOKEN` par défaut n'a que les droits de lecture tant que
  `permissions: contents: write` n'est pas déclaré explicitement) —
  ajouté au workflow.
- Mise à jour des actions GitHub (`checkout`, `upload-artifact`,
  `download-artifact`, `action-gh-release`) vers leurs dernières versions
  majeures (Node 24 natif), pour faire disparaître l'avertissement de
  dépréciation Node 20 dans les logs.

## [1.0.3] - 2026-08-22 (fournisseur OmniRoute, corrections release/mise a jour)
### Ajouts
- **OmniRoute comme fournisseur pour l'assistant IA** ([omniroute.online](https://omniroute.online/),
  [github.com/diegosouzapw/OmniRoute](https://github.com/diegosouzapw/OmniRoute)) :
  passerelle auto-hébergée, compatible OpenAI, donnant accès à 300+
  fournisseurs (Claude, GPT, Gemini, Kimi, DeepSeek...) derrière une seule
  clé API. Champ d'URL dédié (par défaut `http://127.0.0.1:20128/v1`, le
  port local par défaut d'OmniRoute), détection des modèles disponibles via
  `GET /v1/models`, modèle par défaut `auto` (routage intelligent
  zero-config d'OmniRoute). Comme pour les autres fournisseurs, la clé est
  chiffrée sur disque (AES-256-GCM).
### Corrections
- **Le binaire Windows ne contenait ni le binaire CLI headless ni les
  dernières fonctionnalités (assistant IA, etc.).** Deux causes distinctes,
  toutes deux corrigées :
  - Le job Windows du workflow de release compilait avec `cargo build
    --release` (sans `--bins`) et ne copiait que `mcmanager.exe` dans
    l'archive, jamais `mcmanager-headless.exe`. L'installateur Inno Setup
    généré par `build.sh installer` avait le même trou. Les deux copient
    maintenant les deux binaires.
  - Le dépôt GitHub réel est `yo-le-zz/MCmanager`, mais le code (URL de
    mise à jour, user-agent HTTP, liens `nix run`, README, docs, scripts de
    build/installateur) pointait vers `yolezz/mcmanager` — un dépôt
    différent qui n'existe pas. Toute vérification de mise à jour échouait
    donc silencieusement (404 sur l'API GitHub), et le binaire semblait "ne
    jamais se mettre à jour" quel que soit le tag publié. Corrigé partout :
    `src/state.rs` (`update_repo`, user-agent), `src/updater.rs`,
    `README.md`, `docs/BUILD.md`, `docs/TUTO_INSTALLATION_GLFOS.md`,
    `flake.nix`, `packaging/nix/*.nix`, `build.sh`.
- **La création de release GitHub échouait avec une erreur 403** ("Resource
  not accessible by integration"). Le token `GITHUB_TOKEN` par défaut n'a
  que les droits de lecture sur le contenu du dépôt tant qu'on ne déclare
  pas explicitement `permissions: contents: write` dans le workflow —
  ajouté au niveau du workflow et du job `publish`.
- **Avertissement "Node.js 20 is being deprecated"** dans les logs du
  workflow : les actions `actions/checkout`, `actions/upload-artifact`,
  `actions/download-artifact` et `softprops/action-gh-release` étaient
  épinglées sur des versions majeures qui tournent encore sur Node 20.
  Mises à jour vers leurs dernières versions majeures (respectivement v6,
  v7, v8, v3), qui tournent nativement sur Node 24.
- Versions de paquet Nix (`flake.nix`, `packaging/nix/*.nix`) resynchronisées
  avec la version réelle de l'application (elles étaient restées bloquées
  sur d'anciennes valeurs sans rapport avec `Cargo.toml`).

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
- **`nix run github:yo-le-zz/MCmanager` ne fonctionnait pas** : le flake.nix
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

