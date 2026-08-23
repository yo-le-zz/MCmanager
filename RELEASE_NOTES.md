# MCManager v1.0.4 — Notifications, statistiques, liste blanche, console améliorée, et correctifs importants

Cette version ajoute plusieurs nouveaux outils de gestion au quotidien
(notifications, statistiques, liste blanche, propriétés serveur, import/export
de fichiers), améliore nettement la console, et corrige trois bugs qui
empêchaient la v1.0.3 de fonctionner correctement en pratique.

## 🔔 Notifications ntfy (au lieu d'un bot Discord)

Plus simple qu'un bot Discord : pas de compte d'application à créer, juste
un "topic" [ntfy.sh](https://ntfy.sh) (public ou auto-hébergé) auquel
s'abonner depuis l'appli mobile. Alertes disponibles, activables une par
une : crash, sauvegarde terminée, redémarrage programmé, arrêt automatique,
connexion/déconnexion d'un joueur. Jeton d'authentification optionnel,
chiffré sur disque comme les clés IA.

## 📈 Statistiques

Nouvel onglet par serveur : nombre de démarrages, temps de fonctionnement
total, nombre de crashs détectés, et journal des sessions récentes (début,
fin, durée, propre ou crash).

## 🛡 Liste blanche & 📝 Propriétés serveur

Deux nouveaux onglets dédiés :
- **Liste blanche** : ajouter/retirer des joueurs, activer/désactiver
  l'application.
- **Propriétés serveur** : formulaire convivial pour les réglages courants
  de `server.properties` (difficulté, mode de jeu, PvP, max-players,
  distance de vue, mode en ligne...) avec les bons contrôles au lieu de
  texte brut à éditer à la main. Un éditeur "fichier complet" reste
  disponible pour les clés avancées.

## 📟 Console repensée

- **Zone de saisie multi-ligne et redimensionnable** : une commande par
  ligne, bouton "▶ Exécuter" (ou Ctrl+Entrée) pour tout envoyer d'affilée —
  fini le champ trop étroit pour une commande `give` avec NBT custom.
- **Couleurs dans le terminal** : les codes ANSI (Paper/Spigot) et les
  codes couleur Minecraft (`§`) s'affichent maintenant en couleur.
- **Apparence personnalisable** (taille de police, police) dans
  Paramètres, appliquée à toutes les consoles.

## 📦 Import/export de fichiers

Dans l'onglet Fichiers : export d'un dossier (ou de tout le serveur) en
`.zip` téléchargeable, import d'une archive `.zip` avec extraction
protégée contre les chemins malveillants ("zip-slip").

## 🎨 Animations & 🤖 OmniRoute

- Transitions sur la navigation, les cartes, les boutons et les
  notifications toast.
- **OmniRoute** ajouté comme fournisseur pour l'assistant IA — passerelle
  auto-hébergée compatible OpenAI donnant accès à 300+ fournisseurs
  derrière une seule clé.

## 🔒 Verrou d'instance plus intelligent

Au lieu de refuser systématiquement de démarrer si `mcmanager.lock`
existe, MCManager vérifie maintenant si le PID qu'il référence appartient
toujours à un processus vivant, et si c'est bien MCManager. Un verrou
laissé par un arrêt brutal, ou par un PID réutilisé par un autre
programme, est proposé à la suppression de façon interactive (avec refus
automatique en contexte non-interactif, comme un service systemd).

## 🐛 Corrections importantes (suite aux retours sur la v1.0.3)

- **L'installateur Windows n'était jamais généré.** Le workflow le
  compilait sur le job Linux, où Inno Setup n'existe jamais. Déplacé sur
  le job Windows.
- **La mise à jour automatique ne détectait jamais rien.** Le code
  pointait vers un dépôt GitHub qui n'existe pas (`yolezz/mcmanager` au
  lieu de `yo-le-zz/MCmanager`) — chaque vérification échouait
  silencieusement. Corrigé partout.
- **La création de release échouait (403).** Permissions manquantes dans
  le workflow — ajoutées.

## 📥 Téléchargements

| Plateforme | Fichier |
|---|---|
| Linux (Debian/Ubuntu) | `mcmanager_1.0.4_amd64.deb` (inclut `mcmanager` et `mcmanager-headless`) |
| Linux (portable) | `mcmanager-1.0.4-linux-x86_64.tar.gz` |
| NixOS / GLF OS | `nix run github:yo-le-zz/MCmanager` |
| Windows (portable) | `mcmanager-1.0.4-windows-x86_64.zip` |
| Windows (installeur) | `mcmanager-1.0.4-setup.exe` |

**Changelog complet :** voir [CHANGELOG.md](./CHANGELOG.md)

> ⚠️ Comme pour la v1.0.3 : assurez-vous que le tag `v1.0.4` pointe bien
> sur le dernier commit après avoir appliqué ces changements
> (`git push origin v1.0.4`, en recréant le tag s'il existait déjà) pour
> que la release CI compile bien ce code et pas une version antérieure.

## ⏭ Pas encore fait (reporté à une prochaine version)

Pour rester honnête sur le périmètre de cette release : l'authentification
par mot de passe sur l'interface web, le suivi TPS/RCON, la liste des
joueurs en direct avec kick/ban en un clic, et les actions "agissantes" de
l'assistant IA (installer un mod directement depuis une suggestion, etc.)
ne sont pas dans cette version.
