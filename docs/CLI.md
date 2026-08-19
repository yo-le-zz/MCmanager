# CLI headless (gestion depuis un serveur distant sans navigateur)

MCManager expose une petite CLI qui parle à une instance déjà en cours
d'exécution (ex: lancée via systemd sur un VPS) — pratique pour gérer un
serveur Minecraft en SSH sans avoir à ouvrir de navigateur ou de tunnel.

```
mcmanager                       demarre l'interface web (comportement par defaut)
mcmanager serve                 idem, explicite
mcmanager cli list              liste les serveurs enregistres
mcmanager cli status <id>       statut (CPU/RAM/joueurs) d'un serveur
mcmanager cli start <id>        demarre un serveur
mcmanager cli stop <id>         arrete un serveur proprement
mcmanager cli create --name N --loader paper --version 1.21.11 [--port P]
mcmanager --version
mcmanager --help
```

La CLI utilise les mêmes variables d'environnement que le serveur pour le
savoir où le joindre : `MCMANAGER_HOST` (défaut `127.0.0.1`) et
`MCMANAGER_PORT` (défaut `7777`).

## Exemple : installer sur un VPS Ubuntu

```bash
sudo dpkg -i mcmanager_1.0.1_amd64.deb
systemctl --user enable --now mcmanager
# ... plus tard, en SSH :
mcmanager cli list
mcmanager cli status <id>
```

L'`id` d'un serveur s'obtient via `mcmanager cli list` ou dans l'interface
web (URL ou onglet Fichiers).
