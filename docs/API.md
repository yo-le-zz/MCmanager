# API MCManager

Base URL par défaut : `http://127.0.0.1:7777/api`. Toutes les réponses sont en
JSON. Les erreurs renvoient `{"error": "message"}` avec un code HTTP 4xx/5xx.

## Général
- `GET /version` → `{ "version": "1.0.1" }`
- `GET /update/check` → infos de mise à jour (`updater::UpdateInfo`, inclut `install_kind`/`self_update_supported`/`self_update_note`)
- `POST /update/apply` → télécharge et applique la mise à jour (refuse si `.deb`)
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

## Serveurs distants (v1.0.8)
- `GET /servers` → serveurs locaux **et** serveurs des instances jumelées ; ces derniers portent
  `"remote": { "label", "host", "online" }`.
- Toute route `/servers/:id/...` dont l'id appartient à un serveur distant est relayée telle quelle à
  l'instance concernée (console, fichiers, addons, schematics, sauvegardes...). `open-folder` est refusé.
- `POST /servers` et `POST /servers/import` avec l'en-tête `x-mcm-target: <nom de l'instance>` créent/importent
  le serveur sur cette instance.
- `GET /remote/hub` → état de chaque instance jumelée (`online`, `latency_ms`, `last_error`, `server_count`).
- `GET /remote/targets` · `POST /remote/targets` `{host,label,code}` · `DELETE /remote/targets/:label`
- `POST /remote/:label/deploy/:server_id` → envoie un serveur local vers l'instance (par blocs chiffrés).
- WebSocket `/servers/:id/ws` fonctionne aussi pour un serveur distant (relais).

## Mode serveur (`mcmanager-server`)
Hors préfixe `/api` : `POST /auth/login` `{username,password}` · `POST /auth/logout` · `GET /auth/me` ·
`POST /auth/password` · `GET /auth/sessions` · `DELETE /auth/sessions/:id` · `GET|POST /auth/users` ·
`DELETE /auth/users/:name` · `GET /healthz`. Toutes les routes `/api/*` exigent le cookie de session
(ou `Authorization: Bearer <api_token>`).
- `GET /server/info` → empreinte, machines jumelées et sessions actives ; `POST /server/pairing-code`
  `{minutes?}` ; `DELETE /server/clients/:id` (administrateurs).
- `POST /remote/info|pair|session|api` : protocole machine à machine signé RSA, voir `src/remote.rs`.

## Discord et domaines (v1.0.8)
- `GET|POST /discord/config` (token/webhook jamais renvoyés : `has_token`, `has_webhook`) · `GET /discord/status` · `POST /discord/test`
- `GET|PUT /network` : domaines par serveur + configuration générée (DNS, nginx stream, nginx web, Caddy) · `POST /network/detect-ip`
- `GET /servers` ajoute `domain` / `public_port` aux serveurs ayant un domaine.

## Droits et audit (v1.0.9)
- `PUT /auth/users/:name/access` `{default_level, servers:{<uuid>:<level>}}` (administrateurs) ; niveaux : `view` < `console` < `control` < `manage`.
  `GET /auth/users` et `GET /auth/me` renvoient `default_level` et `servers`. `POST /auth/users` accepte les mêmes champs.
- `GET /servers` ne renvoie à un compte non administrateur que ses serveurs, avec `my_level`. Réponse `403` pour toute action non autorisée.
- `GET /audit?limit=&user=&q=&only_denied=` (administrateurs) : journal d'audit, plus récent d'abord.

## Assistant (v1.1.0)
Toutes ces routes sont réservées aux administrateurs.
- `GET|POST /ai/config` : ajoute `tools_mode` (`off` | `read` | `ask` | `auto`, défaut `ask`). Une valeur vide à l'enregistrement conserve le réglage.
- `POST /ai/chat/stream` `{message, history, server_id}` : flux SSE (`text/event-stream`). Chaque événement a un champ `type` :
  `delta {text}` · `tool_start {id, name, label_fr, label_en, args, needs_approval}` · `tool_end {id, name, ok, denied, summary}` · `error {message}` · `done`.
  Fermer la connexion arrête l'agent.
- `POST /ai/approve` `{id, approve}` : répond à un `tool_start` dont `needs_approval` est vrai. Sans réponse en 120 s, l'action est refusée.
- `POST /ai/chat` : enveloppe non streamée, renvoie `{reply, tools}`. Sans interface pour confirmer, le mode `ask` y est traité comme `read`.
