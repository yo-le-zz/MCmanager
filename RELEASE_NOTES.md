# MCManager v1.0.2 — Multilingue, paramètres serveur étendus, CLI headless, assistant IA

Grosse mise à jour : support multilingue complet, gestion avancée des
mods/plugins, un vrai outil de diagnostic de crash, un binaire séparé pour
gérer MCManager sans interface web sur un serveur distant, et un assistant
IA intégré.

## 🌍 Multilingue

- **Français / Anglais / Espagnol** dans toute l'interface, y compris la
  page Docs & tutoriels. Détection automatique de la langue du navigateur,
  sélecteur manuel dans Paramètres.

## 🖥 Interface

- **Mise en page pleine hauteur** : la console (jeu et playit.gg) et
  l'explorateur/éditeur de fichiers occupent tout l'espace vertical
  disponible au lieu d'une hauteur fixe, quel que soit le format d'écran.
- **Bouton "Ouvrir dans l'explorateur"** sur chaque serveur et dans
  Fichiers : ouvre le dossier correspondant dans l'explorateur natif de
  l'OS.
- **Bouton "Config" par mod/plugin** : accès direct au dossier
  `mods/`/`plugins/` du serveur depuis l'onglet Fichiers.
- Suppression des préréglages recommandés dans Mods/Plugins (prenaient de
  la place sans utilité avérée).

## ⚙️ Paramètres serveur étendus

- Délai de redémarrage après crash configurable (au lieu d'un délai fixe
  de 5s).
- Redémarrage programmé (toutes les N minutes).
- Arrêt automatique si aucun joueur n'a rejoint depuis N minutes.
- Tout est modifiable à tout moment depuis Paramètres, même après la
  création du serveur, sans attendre un redémarrage manuel.

## 🧩 Mods/plugins avancés

- **Bouton "Ajouter les mods/plugins de performance"** : installe en un
  clic un trousseau curé (Chunky, Lithium, spark...) compatible avec le
  loader du serveur.
- **Mods/plugins "gérés"** : liste définie par l'utilisateur (par ID/slug
  Modrinth) que MCManager garde dans la bonne version pour ce serveur.
  Rien ne se télécharge automatiquement — un bouton "Synchroniser
  maintenant" déclenche la mise à jour quand l'utilisateur le décide.
  Ajoutable aussi directement depuis les résultats du Marketplace.

## 🩺 Mode debug — diagnostic de crash

Nouveau bouton dans la Console qui automatise la recherche d'un mod/plugin
qui fait planter le serveur :

1. Teste d'abord la configuration actuelle telle quelle (rien n'est
   touché si elle démarre déjà).
2. Sinon, désactive tout, confirme que le serveur démarre nu, puis
   réactive chaque addon individuellement pour trouver celui qui plante
   **à lui seul**.
3. Si aucun coupable individuel n'est trouvé mais que l'ensemble complet
   ne démarre pas, réactive les addons un par un de façon cumulative pour
   repérer une **combinaison** problématique entre plusieurs d'entre eux.

Tout est remis dans l'état d'origine à la fin, et la progression s'affiche
en direct dans la console.

## 🖧 Binaire CLI séparé — `mcmanager-headless`

Gère les serveurs (création, démarrage/arrêt, mods/plugins, diagnostic...)
entièrement en ligne de commande, **sans jamais démarrer de serveur
web/API HTTP** — pensé pour un VPS/serveur Ubuntu sans navigateur. Partage
le même code et le même format de données que `mcmanager` (web). Un
verrou d'instance (`mcmanager.lock`) empêche de lancer les deux binaires
en même temps sur le même dossier de données et de corrompre l'état des
serveurs. Voir [docs/HEADLESS.md](./docs/HEADLESS.md).

## 🤖 Assistant IA

Nouvel onglet "Assistant IA" : chatbox qui donne des suggestions sur quoi
ajouter/modifier/réparer, avec le contexte réel du serveur sélectionné
(loader, version, mods/plugins installés, état).

- Quatre fournisseurs au choix : **Anthropic (Claude), OpenAI, Google
  Gemini, ou Ollama en local**.
- Le fournisseur est détecté automatiquement à partir du format de la clé
  collée, et la liste des modèles disponibles est récupérée directement
  auprès du fournisseur.
- **Clé API chiffrée sur disque (AES-256-GCM)**, avec une clé de
  chiffrement générée localement et stockée séparément (accès restreint
  au propriétaire du compte). Une vraie protection contre une
  copie/sauvegarde accidentelle du seul fichier de config — mais pas
  l'équivalent d'un trousseau système, puisque la clé de déchiffrement
  reste sur la même machine (indiqué clairement dans l'interface).
- Pour **Ollama en local uniquement**, l'assistant dispose d'outils de
  recherche web et de lecture de page (pour compenser l'absence de
  connaissances à jour d'un modèle local), avec repli automatique sur
  Wikipedia si la recherche web échoue.

## 🐛 Corrections

- **playit.gg n'affichait rien au démarrage tant que le binaire était déjà
  téléchargé** : la sortie de l'agent imprimée juste avant l'ouverture du
  WebSocket de la console était perdue. Un tampon de rediffusion a été
  ajouté (même principe que la console des serveurs) : les dernières
  lignes sont maintenant rejouées à la connexion.

## 📥 Téléchargements

| Plateforme | Fichier |
|---|---|
| Linux (Debian/Ubuntu) | `mcmanager_1.0.2_amd64.deb` (inclut `mcmanager` et `mcmanager-headless`) |
| Linux (portable) | `mcmanager-1.0.2-linux-x86_64.tar.gz` |
| NixOS / GLF OS | `nix run github:yolezz/mcmanager` |
| Windows (portable) | `mcmanager-1.0.2-windows-x86_64.zip` |
| Windows (installeur) | `mcmanager-1.0.2-setup.exe` |

**Changelog complet :** voir [CHANGELOG.md](./CHANGELOG.md)
