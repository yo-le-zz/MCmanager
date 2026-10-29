# MCManager v1.0.9 — Bot Discord complet, domaines, paquets .rpm

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
