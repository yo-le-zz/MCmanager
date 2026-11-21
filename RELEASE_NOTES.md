# MCManager v1.1.3 — Bedrock, serveurs toujours allumés, transferts entre machines

## Nouveautés
- **Minecraft Bedrock** : serveur officiel (x86_64), Preview, **PocketMine-MP** et **Nukkit** (ARM compris) ; packs, add-ons et mondes `.mcworld`, liste d'autorisation, sauvegarde à chaud ([docs/BEDROCK.md](docs/BEDROCK.md)).
- **Reprise des serveurs** : ils redémarrent seuls avec MCManager, y compris après une mise à jour ; service de démarrage en une commande (`--install-service`).
- **Transferts entre machines** reprenables et vérifiés (SHA-256) : copier, déplacer, rapatrier, miroir des sauvegardes ; mise à jour d'une machine distante.
- **Modèles de serveur** : recréer un serveur sans rien télécharger.
- **HTTPS natif** et **migration automatique** des anciennes données (avec copie de sécurité).
- **Discord** est maintenant une entrée du menu, plus un réglage.

## À savoir
- Le serveur Bedrock officiel n'existe que pour x86_64 (Linux, Windows) : sur Raspberry Pi, utilisez PocketMine-MP ou Nukkit.
- playit.gg : MCManager ne fait que lancer l'agent ; pour Bedrock, créez un tunnel **UDP** dans votre compte playit (voir [docs/PLAYIT.md](docs/PLAYIT.md), non testé avec un vrai compte).
- Non exécuté avant la publication : workflows GitHub, Docker, builds macOS / Windows / ARM, téléchargements Bedrock réels, mise à jour distante réelle (voir le CHANGELOG).


# MCManager v1.1.2 — Stabilisation : Discord, double authentification, alertes, mobile

## Nouveautés
- **Bot Discord** : statut et activité modifiables à chaud, lien pour inviter le bot, commande `/mc rejoindre` (adresse à coller dans Minecraft). Webhook toujours possible sans bot.
- **Double authentification (TOTP)** avec codes de secours ([docs/SECURITY.md](docs/SECURITY.md)).
- **Alertes** sur le tableau de bord : disque, sauvegardes, EULA, bot Discord.
- **Menu mobile** pour téléphone.

## Sécurité et fiabilité
- Mise à jour automatique : empreinte SHA-256 vérifiée avant installation.
- Imports de zip plafonnés. Assistant : flux gardé ouvert pendant les confirmations.
- Interface testée dans un vrai navigateur (Firefox headless, aussi en CI).


# MCManager v1.1.1 — macOS, compatibilité, mise à jour corrigée

## Nouveautés
- **macOS** : Mac Intel, `MCManager.app` dans un `.dmg` (macOS 11+) ; Apple Silicon via Rosetta 2. Non notarisée : au premier lancement, clic droit → Ouvrir ([docs/MACOS.md](docs/MACOS.md)).
- **Détection de Java** dans les réglages, avec la version requise par votre version de Minecraft.
- **Linux** : binaires compatibles Debian 12, Raspberry Pi OS Bookworm, RHEL 9 et plus récents ; `install.sh` gère x86_64, ARM64, `.deb`, `.rpm` et macOS.
- Fichier **SHA256SUMS** dans la release.

## Correctifs
- La **mise à jour automatique** remplaçait l'exécutable par l'archive téléchargée et laissait l'interface d'ancienne version : corrigé (exécutables + `web/`, retour arrière en cas d'échec).
- Sauvegardes et exports zip **en flux** : plus de pic de mémoire sur les gros mondes.
- Interface servie compressée.

## Version précédente — v1.1.0
## Fond fluide, couleurs libres, assistant avec outils

## Interface
- **Fond animé plus fluide** : rendu par zones, cartes opaques, réglage de qualité avec dégradation automatique.
- **Thème Personnalisé** : 14 couleurs libres, dérivation depuis fond + texte, accent hexadécimal, export / import JSON validé, CSS libre.

## Assistant
- Réponses **en direct**, bouton **Arrêter**, pastilles d'outils (en cours / en attente / fait / erreur).
- **Outils pour les cinq fournisseurs** ; les modifications demandent votre **accord** (mode `ask`, défaut) et sont inscrites au journal d'audit.
- Mode des outils : désactivés, lecture, confirmer, automatique.

## Sécurité
- `web_fetch` ne peut plus atteindre le réseau local ni les adresses de métadonnées (SSRF corrigé).

## Version précédente — MCManager v1.0.9 — Bot Discord complet, domaines, paquets .rpm

## 🤖 Bot Discord
- **Une catégorie par serveur Minecraft**, créée par le bot : salon **statut**, **console en direct** (les rôles autorisés
  y tapent des commandes) et **chat** relié au jeu dans les deux sens.
- **Détection automatique** des serveurs Discord, rôles et salons (cases à cocher, menus) : plus d'identifiants à recopier.
- **Version anglaise** : le bot et les réglages parlent français ou anglais ; les commandes `/mc` suivent la langue de chaque utilisateur.
- Sécurité : salons invisibles pour `@everyone`, droits revérifiés par MCManager, aucun ping possible, refus par défaut.

## 🔗 Domaines personnalisés
Onglet **Domaines** : DNS (A/SRV), nginx (proxy TCP, interface web + WebSocket + HTTPS) et Caddy générés pour vos serveurs.

## 📦 Paquets `.rpm`
Fedora, RHEL/Rocky/Alma, openSUSE : `.rpm` x86_64 et aarch64, mêmes binaires et mêmes fichiers que le `.deb`.
`sudo dnf install ./mcmanager-1.0.9-1.x86_64.rpm`

## 📥 Import / export
Importer un serveur depuis une archive `.zip` ; bouton **Exporter** sur chaque serveur.

## 🛠 Corrections
Publication GitHub (artefact de trace), boîtes de confirmation bloquables par le navigateur, `.deb` sans dépendance `libssl`.
