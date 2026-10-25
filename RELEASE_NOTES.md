# MCManager v1.0.8 — Serveurs distants intégrés, mode serveur, Docker

## 🖧 Les serveurs distants sont dans votre tableau de bord
Jumelez une autre machine MCManager : ses serveurs apparaissent **comme les vôtres**, avec une étiquette
« distant ». Console en direct, fichiers, mods/plugins, schematics, sauvegardes, démarrage/arrêt — tout
fonctionne à l'identique, exécuté sur la machine distante. Création directe là-bas via « Héberger sur ».

## 🔴 Corrections critiques
- `remote enable` ne s'éteint plus avec le terminal (`remote install-permanent`, service permanent).
- Plusieurs utilisateurs/appareils en même temps ne se déconnectent plus mutuellement.
- Requêtes à usage unique (anti-rejeu), ouverture de session signée.
- Téléversements > 2 Mo (mods, schematics, archives) enfin acceptés.

## 🐳 Mode serveur
`mcmanager-headless` devient **`mcmanager-server`** : interface web complète avec connexion multi-utilisateurs,
service systemd durci, **image Docker** (`docker compose up -d`). Voir `docs/SERVER.md`.

## ➖ Retiré
Support Nix (on se concentre sur Linux).

## ⚠ À savoir
Service `mcmanager-headless` remplacé par `mcmanager-server` (désactivé automatiquement par le .deb ; ses données
sont à copier dans `/var/lib/mcmanager`). Transferts plafonnés à 512 Mo.
