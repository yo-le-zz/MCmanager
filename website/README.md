# website/ — mcmanager.pages.dev

Site vitrine statique pour MCManager, basé sur la maquette générée avec
Stitch (voir le zip fourni : `DESIGN.md` pour le système de design,
`code.html` pour la maquette d'origine — `index.html` ici en est
l'adaptation avec des liens et commandes réels).

## Déploiement (Cloudflare Pages)

Aucune étape de build : c'est du HTML statique (Tailwind chargé depuis son
CDN, comme dans la maquette d'origine).

1. Sur [pages.cloudflare.com](https://pages.cloudflare.com), créer un
   projet Pages pointant sur ce dossier `website/` (soit en connectant le
   dépôt GitHub avec *Build output directory* = `website`, soit via
   `wrangler pages deploy website`).
2. Nom du projet : `mcmanager` (donne `mcmanager.pages.dev`), ou associer
   un domaine personnalisé dans les réglages du projet.
3. Aucune variable d'environnement ni commande de build nécessaire.

## Contenu

- `index.html` — la page elle-même.
- `assets/logo.svg`, `assets/icon-256.png`, `assets/favicon-32.png` —
  repris directement de `web/assets/` (même icône que l'application, pour
  la cohérence de marque).
- `install.sh` — servi à `mcmanager.pages.dev/install.sh`, référencé par
  la commande `curl -fsSL mcmanager.pages.dev/install.sh | bash` affichée
  sur le site (carte Linux).
- `install.ps1` — servi à `mcmanager.pages.dev/install.ps1`, référencé par
  la commande `iex (irm mcmanager.pages.dev/install.ps1)` (carte Windows).

Ces deux scripts sont identiques à ceux du dossier `/scripts` à la racine
du dépôt — dupliqués ici uniquement parce qu'ils doivent être servis
publiquement à ces URLs précises pour que les commandes affichées sur le
site fonctionnent telles quelles. Modifiez l'un, reportez le changement
dans l'autre (ou remplacez par un lien symbolique si votre pipeline de
déploiement le permet).

## Mettre à jour les liens de téléchargement

Le bouton Windows et le texte du site pointent vers
`https://github.com/yo-le-zz/MCmanager/releases/latest` (toujours la
dernière version, pas de numéro de version à modifier à chaque release).
Les scripts `install.sh`/`install.ps1` résolvent eux aussi la dernière
release automatiquement via l'API GitHub — rien à mettre à jour ici lors
d'une nouvelle sortie.
