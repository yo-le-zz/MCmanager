# API MCManager

Base URL par défaut : `http://127.0.0.1:7777/api`. Toutes les réponses sont en
JSON. Les erreurs renvoient `{"error": "message"}` avec un code HTTP 4xx/5xx.

## Général
- `GET /version` → `{ "version": "1.0.1" }`
- `GET /update/check` → infos de mise à jour (`updater::UpdateInfo`, inclut `install_kind`/`self_update_supported`/`self_update_note`)
- `POST /update/apply` → télécharge et applique la mise à jour (refuse si `.deb`/Nix)
- `GET /settings` / `PUT /settings` → configuration globale (chemin Java, dépôt GitHub...)
- `GET /loaders/:loader/versions` → liste des versions Minecraft dispo pour ce loader
- `GET /loaders/:loader/builds?version=` → liste des builds/loader-versions installables pour cette version
- `GET /presets` → préréglages "un clic" disponibles

## Serveurs
- `GET /servers` / `POST /servers` → liste / création (télécharge et configure le serveur)
- `POST /servers/import` → enregistre un serveur déjà présent sur le disque
- `GET /servers/:id` / `PUT /servers/:id` / `DELETE /servers/:id` (suppression = dossier + sauvegardes)
- `POST /servers/:id/start` / `stop` / `kill`
- `POST /servers/:id/command` `{ "cmd": "say bonjour" }`
- `GET /servers/:id/status` → CPU, RAM, joueurs en ligne
- `GET /servers/:id/ws` → WebSocket console (backlog puis flux temps réel ; envoyer du texte = commande)
- `POST /servers/:id/console/clear` → vide le tampon de console côté serveur

## Fichiers
- `GET /servers/:id/files?path=` → liste un dossier
- `GET/PUT /servers/:id/files/content?path=` → lecture/écriture d'un fichier texte
- `POST /servers/:id/files/upload` → upload multipart (`path` = dossier destination, `file` = contenu)
- `DELETE /servers/:id/files?path=`

## Mods / Plugins
- `GET /servers/:id/addons`
- `POST /servers/:id/addons/:file/toggle`
- `DELETE /servers/:id/addons/:file`
- `POST /servers/:id/presets/:key/install`
- `GET /servers/:id/marketplace/updates` → mises à jour disponibles pour les addons installés

## Marketplace (Modrinth)
- `GET /marketplace/search?q=&type=mod|plugin&loader=&version=`
- `GET /marketplace/project/:id/versions?loader=&version=`
- `POST /servers/:id/marketplace/install` `{ "project_id": "...", "version_id": "optionnel" }`

## Schematics
- `GET /servers/:id/schematics`
- `POST /servers/:id/schematics` → upload multipart (`.schem`/`.schematic`)
- `DELETE /servers/:id/schematics/:file`

## Sauvegardes
- `GET /servers/:id/backups`
- `POST /servers/:id/backups` → démarre une sauvegarde en tâche de fond (`{ "started": true, "total": N }`)
- `GET /servers/:id/backups/progress` → `{ "running": bool, "done": N, "total": N }`
- `POST /servers/:id/backups/:name/restore`
- `DELETE /servers/:id/backups/:name`

## playit.gg
- `POST /playit/download` / `start` / `stop`
- `GET /playit/status`
- `POST /playit/detect-local` → détecte et adopte une installation `playit` déjà présente sur la machine
- `GET /playit/ws` → WebSocket des logs de l'agent
