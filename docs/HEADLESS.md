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

## Démarrage automatique de serveurs

`autostart add <id>` marque un serveur pour qu'il démarre automatiquement
à chaque lancement de `mcmanager-headless` (par exemple après un reboot,
via le service systemd ci-dessous) — pas besoin de taper `start` à la main.

```
mcmanager> autostart add 7c3a1e2e-...-...
mcmanager> autostart list
```

`autostart remove <id>` désactive, `autostart list` affiche la liste actuelle.

## Contrôle à distance (chiffré, authentifié par clé RSA)

`mcmanager-headless` peut exposer une petite API de contrôle sur le réseau
(`0.0.0.0`, toutes interfaces) pour être piloté depuis une autre machine —
**strictement optionnel, désactivé par défaut**. Chiffrement hybride
RSA + AES-256-GCM (comme TLS/SSH/PGP : RSA échange une clé de session
AES, qui chiffre ensuite les échanges), et chaque requête est signée par
la clé privée du client pour authentifier qui la fait.

**Sur la machine à piloter** (celle qui héberge les serveurs) :
```
mcmanager> remote enable 7778
Contrôle à distance actif sur le port 7778 (0.0.0.0 - toutes interfaces).
Empreinte de cette instance : d4:41:52:0b:...

mcmanager> remote pairing-code
Code de jumelage (valide 10 minutes, usage unique) : 03847680
```

**Sur la machine qui pilote** (peut être une autre install de
`mcmanager-headless`, y compris sur votre PC) :
```
mcmanager> remote pair 192.168.1.42:7778 mon-serveur
Empreinte annoncée par 192.168.1.42:7778 : d4:41:52:0b:...
Vérifiez qu'elle correspond à celle affichée par 'remote pairing-code' sur cette machine distante.
Code de jumelage reçu de cette machine : 03847680
Jumelé avec succès sous le nom "mon-serveur".

mcmanager> remote list mon-serveur
mcmanager> remote start mon-serveur 7c3a1e2e-...-...
mcmanager> remote status mon-serveur 7c3a1e2e-...-...
mcmanager> remote send mon-serveur 7c3a1e2e-...-... say bonjour
```

Le jumelage exige de lire le code affiché sur la machine hébergeant les
serveurs — un inconnu qui trouve juste le port ouvert ne peut pas
s'auto-jumeler. Gestion des accès : `remote clients` (qui est autorisé),
`remote revoke <id>` (retirer un accès), `remote disable` (tout couper).

## Mode daemon (systemd, ou tout superviseur de process)

Sans terminal attaché, le mode interactif normal se termine immédiatement
(l'entrée standard est `/dev/null`, donc lue comme "fin de fichier" tout de
suite). Le flag `--daemon` fait attendre le process indéfiniment (jusqu'à
Ctrl+C / SIGTERM) au lieu de quitter - combiné à `--script`, ça permet de
lancer des commandes de configuration puis de rester actif :

```
mcmanager-headless --script /etc/mcmanager/autostart.txt --daemon
```

Le service systemd fourni par le paquet `.deb` (`mcmanager-headless.service`,
unité **système**, pas utilisateur - pour démarrer avant toute connexion)
utilise exactement cette combinaison, avec `Restart=on-failure` :

```
sudo systemctl enable --now mcmanager-headless   # demarre maintenant + a chaque boot
sudo systemctl status mcmanager-headless
```

Le fichier `/etc/mcmanager/autostart.txt` (une commande par ligne) permet
d'y activer `remote enable`, ou tout autre réglage à appliquer au
démarrage - il peut aussi rester vide : le démarrage automatique configuré
via `autostart add <id>` s'applique de toute façon à chaque lancement,
indépendamment de ce fichier.
