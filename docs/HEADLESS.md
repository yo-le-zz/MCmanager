# mcmanager-headless (aucun serveur web, tout en CLI)

`mcmanager-headless` est un **binaire séparé** de `mcmanager` : il gère les
serveurs Minecraft (création, démarrage/arrêt, mods/plugins, diagnostic de
crash...) directement en ligne de commande, **sans jamais ouvrir de port web
ni démarrer d'API HTTP**. Pensé pour un VPS/serveur Ubuntu où vous ne voulez
ni navigateur, ni tunnel SSH, ni port supplémentaire à ouvrir.

> Différence avec `mcmanager cli` (voir [docs/CLI.md](CLI.md)) : cette
> dernière est une télécommande qui parle en HTTP à une instance `mcmanager`
> (web) déjà lancée — le serveur web tourne quand même en arrière-plan.
> `mcmanager-headless` ne lance **aucun** serveur web, à aucun moment.

## Utilisation

```bash
./mcmanager-headless
```

Lance un shell interactif :

```
MCManager v1.0.2 - mode CLI (aucun serveur web ne sera lance)
Dossier de donnees : /home/user/.local/share/mcmanager
Tapez 'help' pour la liste des commandes, 'quit' pour quitter.

mcmanager> help
Commandes disponibles :
  list                          liste les serveurs enregistres
  status <id>                   CPU / RAM / joueurs en ligne
  start <id>                    demarre un serveur
  stop <id> [--force]           arrete (proprement, ou --force pour tuer le processus)
  restart <id>                  arrete puis redemarre
  logs <id> [n]                 affiche les n dernieres lignes de la console (defaut 40)
  send <id> <commande...>       envoie une commande a la console du serveur
  install <id> <slug|id>        installe un mod/plugin Modrinth (derniere version compatible)
  managed-sync <id>             synchronise les mods/plugins "geres" de ce serveur
  debug <id>                    diagnostic automatique de crash
  create --name N --loader L --version V [--port P]
  help
  quit / exit
```

Ou en mode script (utile pour de l'automatisation/cron) :

```bash
./mcmanager-headless --script commandes.txt
```

Où `commandes.txt` contient une commande par ligne (les lignes vides et
commençant par `#` sont ignorées).

## Pourquoi un process persistant plutôt que des commandes isolées ?

La supervision des processus serveur (savoir qu'un serveur tourne, lire sa
console en direct, gérer les redémarrages) vit en mémoire pendant toute la
durée de vie du process `mcmanager-headless` — exactement comme pour
`mcmanager` (web). Lancer `mcmanager-headless start <id>` puis quitter
immédiatement laisserait le serveur Minecraft tourner "orphelin", sans que
plus personne ne surveille sa console ou ne gère un crash. Le shell
interactif (ou le mode `--script`) reste donc ouvert : vous y tapez vos
commandes au fur et à mesure.

## Un seul à la fois sur un même dossier de données

`mcmanager` (web) et `mcmanager-headless` partagent exactement le même
format de données (`servers.json`, dossiers de serveurs...) — un serveur créé
avec l'un est visible et gérable depuis l'autre. **Mais pas simultanément** :
chacun gère les process serveur uniquement en mémoire, donc lancer les deux
en même temps sur le même dossier de données risque de démarrer un serveur
en double ou de corrompre `servers.json`.

Un verrou (`mcmanager.lock` dans le dossier de données) empêche ce cas : si
une instance tourne déjà, la seconde refuse de démarrer avec un message
explicite. Le verrou est nettoyé automatiquement à la fermeture propre
(Ctrl+D, `quit`, Ctrl+C, SIGTERM) — seul un arrêt brutal (`kill -9`, coupure
de courant) peut le laisser en place ; dans ce cas, supprimez-le manuellement
avant de relancer, comme indiqué dans le message d'erreur.

## Variables d'environnement

Les mêmes que pour `mcmanager` (web) : `MCMANAGER_DATA_DIR`. `MCMANAGER_HOST`
et `MCMANAGER_PORT` n'ont aucun effet ici puisqu'aucun serveur web n'est
démarré.

## Exemple : service systemd sur un VPS

```ini
# /etc/systemd/system/mcmanager-headless.service
[Unit]
Description=MCManager (CLI, sans interface web)
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/mcmanager-headless --script /etc/mcmanager/autostart.txt
Restart=on-failure
User=minecraft

[Install]
WantedBy=multi-user.target
```

Où `autostart.txt` contiendrait par exemple :
```
start 7c3a1e2e-...-...
```
