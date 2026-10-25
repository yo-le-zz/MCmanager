# Mode serveur — `mcmanager-server` (VPS, Raspberry Pi, Docker)

`mcmanager-server` est MCManager pensé pour tourner **sans écran, en continu, et
être utilisé à distance**. C'est le même cœur que l'application de bureau
(`mcmanager`) : mêmes serveurs, même format de données, **la même interface
web** — mais avec connexion obligatoire, plusieurs utilisateurs simultanés et
le contrôle à distance par d'autres MCManager.

| | `mcmanager` (bureau) | `mcmanager-server` |
|---|---|---|
| Écoute par défaut | `127.0.0.1:7777` | `0.0.0.0:7777` |
| Connexion | aucune (local) | **obligatoire**, multi-comptes |
| Ouvre le navigateur | oui | non |
| Contrôle à distance (`/remote/*`) | non (il est *client*) | oui, sur le même port |
| Pensé pour | votre PC | VPS, serveur maison, Pi, Docker |

Il se pilote de trois façons, au choix : l'interface web, le shell interactif
en SSH, ou un fichier de commandes.

## Démarrage rapide

```bash
mcmanager-server --daemon
```

Au premier lancement un compte administrateur est créé et son mot de passe
**s'affiche une seule fois** dans la console (ou les logs : `journalctl -u
mcmanager-server`). Ouvrez `http://<adresse-de-la-machine>:7777`, connectez-vous,
changez le mot de passe dans l'onglet **Utilisateurs**.

Pour choisir vous-même le compte initial :

```bash
MCMANAGER_ADMIN_USER=moi MCMANAGER_ADMIN_PASSWORD='un-mot-de-passe-long' mcmanager-server --daemon
```

> ⚠ Sans HTTPS, mot de passe et cookie de session circulent en clair. Sur
> Internet, placez MCManager derrière un reverse proxy HTTPS (Caddy, nginx,
> Traefik) et définissez `MCMANAGER_TRUST_PROXY=1` + `MCMANAGER_COOKIE_SECURE=1`.
> Sur un réseau local de confiance ou via un VPN (WireGuard, Tailscale), le HTTP
> simple est acceptable.

## Service permanent (systemd)

Avec le paquet `.deb` :

```bash
sudo systemctl enable --now mcmanager-server
sudo journalctl -u mcmanager-server        # mot de passe admin initial, logs
```

Le service tourne sous l'utilisateur dédié `mcmanager` (pas root), données dans
`/var/lib/mcmanager`, redémarre tout seul après un plantage ou un reboot, et
laisse 90 s aux serveurs Minecraft pour sauvegarder le monde à l'arrêt.

- `/etc/mcmanager/server.env` — port, hôte, compte admin initial, options proxy.
- `/etc/mcmanager/autostart.txt` — une commande par ligne exécutée au démarrage
  (ex. `start <id>`).

> Si vous utilisiez le service `mcmanager-headless` (avant 1.0.8) : il est
> remplacé par `mcmanager-server` et désactivé automatiquement à l'installation.
> Ses données étaient dans le dossier de l'utilisateur qui le lançait ; copiez-les
> dans `/var/lib/mcmanager` (`chown -R mcmanager:mcmanager`) pour les reprendre.

## Docker

```bash
docker compose up -d        # voir docker-compose.yml à la racine du dépôt
docker logs mcmanager       # mot de passe admin initial si non fixé
```

ou

```bash
docker build -t mcmanager .
docker run -d --name mcmanager --restart unless-stopped \
  -p 7777:7777 -p 25565:25565 -v mcmanager-data:/data \
  -e MCMANAGER_ADMIN_USER=admin -e MCMANAGER_ADMIN_PASSWORD='mot-de-passe-long' \
  mcmanager
```

- **Tout est dans le volume `/data`** : serveurs, mondes, sauvegardes, comptes,
  clés. Sauvegardez-le.
- **Ports Minecraft** : publiez avec `-p` le port de chaque serveur que vous créez
  (25565 par défaut). Alternative sur Linux : `network_mode: host`.
- **Java** : l'image contient Java 21 (défaut) et Java 17 (`/opt/java/17/bin/java`,
  à indiquer comme « chemin Java » d'un serveur pour les Minecraft 1.17 → 1.20.4).
- `docker stop` arrête proprement les serveurs Minecraft avant de couper.
- Gérer les comptes : `docker exec -it mcmanager mcmanager-server user passwd admin`.

## Plusieurs utilisateurs en même temps

Chaque personne a son compte et peut se connecter depuis autant d'appareils
qu'elle veut. Onglet **Utilisateurs** : voir qui est connecté (adresse, navigateur,
dernière activité), déconnecter une session, changer son mot de passe ; les
administrateurs créent et suppriment les comptes.

- La console d'un serveur est partagée : tout le monde voit les mêmes lignes en direct.
- L'édition de fichiers est « dernier enregistrement gagnant » : évitez d'éditer le
  même fichier à deux.
- Mots de passe : PBKDF2-SHA256 (310 000 tours), jamais stockés en clair. Sessions
  par cookie `HttpOnly` + `SameSite=Strict`, 12 h d'inactivité. Après 8 échecs de
  connexion, l'adresse est bloquée 10 min.
- Seuls les administrateurs peuvent jumeler une machine ou gérer les comptes.

En ligne de commande (fonctionne même pendant que le service tourne) :

```bash
mcmanager-server user list
mcmanager-server user add alice [--admin]
mcmanager-server user passwd alice
mcmanager-server user remove alice
```

## Voir les serveurs d'autres machines comme les vôtres

C'est le but du **contrôle à distance** : depuis *n'importe quel* MCManager (votre
PC, un autre serveur), vous jumelez une instance `mcmanager-server`, et **ses
serveurs apparaissent dans votre tableau de bord comme les vôtres**, avec une
étiquette `🖧 distant`. Console en direct, fichiers, mods/plugins, marketplace,
schematics, sauvegardes, propriétés, démarrage/arrêt : tout fonctionne à
l'identique — l'action est simplement exécutée sur la machine distante.
L'assistant de création propose « Héberger sur » pour créer un serveur directement
là-bas.

### Jumeler

1. **Sur la machine distante** : interface web → **Contrôle à distance** →
   *Générer un code de jumelage* (ou, en shell : `remote pairing-code`). Code à
   usage unique, valable 10 minutes.
2. **Sur votre MCManager** : **Contrôle à distance** → *Jumeler une nouvelle
   instance* → adresse `hôte:7777`, un nom, le code.
3. Vérifiez que l'empreinte affichée correspond à celle de la machine distante.

### Sécurité

- Échange de clés RSA-2048, requêtes chiffrées AES-256-GCM et **signées** par
  le client, fenêtre de fraîcheur de 60 s, **nonce à usage unique** (une requête
  capturée ne peut pas être rejouée). Aucun accès sans jumelage préalable.
- Plusieurs machines peuvent être jumelées en même temps, chacune avec autant de
  sessions simultanées que nécessaire ; `remote clients` / l'onglet montrent qui est
  connecté et quand ; `remote revoke` coupe l'accès immédiatement.
- Une machine jumelée ne peut atteindre que la gestion des serveurs
  (`/api/servers`, loaders, marketplace, préréglages) : ni les réglages, ni les
  clés d'IA, ni la mise à jour de l'application.
- Pas de chaînage : une instance ne ré-exporte pas les serveurs d'une autre.

### Limites connues

- Les transferts (upload d'un mod, d'un monde, export d'un dossier) passent en
  mémoire et par blocs chiffrés : plafond de **512 Mo** par opération. Pour un
  très gros monde, utilisez rsync/scp.
- « Ouvrir dans l'explorateur » est refusé pour un serveur distant (la fenêtre
  s'ouvrirait chez l'autre) : utilisez l'onglet Fichiers.
- Si la machine distante tombe, ses serveurs restent listés avec « hors ligne » et
  les actions échouent proprement ; tout revient à la reconnexion (rafraîchissement
  toutes les 5 s).
- La console distante a ~1 s de latence au pire (lecture longue durée), pas de
  WebSocket direct.

## Shell interactif et scripts

```bash
mcmanager-server                          # shell interactif (SSH)
mcmanager-server --script commandes.txt   # exécute puis quitte
mcmanager-server --script commandes.txt --daemon   # exécute puis reste actif (c'est ce que fait le service)
mcmanager-server --daemon --no-web        # pas d'interface web (contrôle à distance et démarrage auto seulement)
```

Tapez `help` dans le shell pour la liste complète : `list`, `status`, `start`,
`stop`, `restart`, `logs`, `send`, `install`, `managed-sync`, `debug`, `create`,
`autostart`, `user`, `remote ...`.

> Un seul processus MCManager par dossier de données (verrou `mcmanager.lock`) :
> pendant que le service tourne, le shell interactif ne peut pas démarrer — utilisez
> l'interface web, `mcmanager cli ...` (voir [CLI.md](CLI.md)) ou
> `mcmanager-server user ...`.

### Contrôle à distance sans interface web

`remote enable [port]` ouvre un port dédié (7778) pour cette session ;
`remote install-permanent [port]` l'installe comme service. En mode `--daemon`
avec interface web, ce n'est pas nécessaire : le contrôle à distance est déjà
actif sur le port web.

## Variables d'environnement

| Variable | Défaut | Rôle |
|---|---|---|
| `MCMANAGER_HOST` | `0.0.0.0` | adresse d'écoute |
| `MCMANAGER_PORT` | `7777` | port de l'interface web et du contrôle à distance |
| `MCMANAGER_DATA_DIR` | `~/.local/share/mcmanager` | dossier de données |
| `MCMANAGER_AUTH` | `on` | `off` désactive la connexion (⚠ à n'utiliser que sur une interface locale) |
| `MCMANAGER_ADMIN_USER` / `_PASSWORD` | — | compte administrateur créé s'il n'y a aucun compte |
| `MCMANAGER_TRUST_PROXY` | non | `1` : lit l'adresse client dans `X-Forwarded-For` (reverse proxy) |
| `MCMANAGER_COOKIE_SECURE` | non | `1` : cookie de session `Secure` (HTTPS) |
| `MCMANAGER_TOKEN` | — | jeton d'API pour `mcmanager cli` (sinon lu dans `<data>/api_token`) |
