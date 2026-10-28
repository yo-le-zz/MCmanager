# Domaines personnalisés et redirections (nginx / Caddy / DNS)

Onglet **🔗 Domaines**. MCManager **génère** la configuration ; il ne modifie jamais lui-même nginx ni le DNS (écrire
dans `/etc/nginx` demande root, et une config cassée peut couper un site entier). Vous relisez, copiez, appliquez.

## Jouer sur `play.exemple.fr` — sans proxy (recommandé)

Minecraft Java lit les enregistrements DNS **SRV**. Pour chaque serveur : un domaine, mode « DNS seul ».

```
play.exemple.fr.                    300 IN A    203.0.113.7
_minecraft._tcp.creatif.exemple.fr. 300 IN SRV  0 5 25570 creatif.exemple.fr.
```

Les joueurs tapent `creatif.exemple.fr` **sans port**, même si le serveur écoute sur 25570. Plusieurs serveurs sur une
même IP : un sous-domaine chacun. Il reste à ouvrir/rediriger le port TCP du serveur sur votre box.

## Proxy TCP nginx (cas particuliers)

Mode « Proxy nginx » : port public différent du port réel, ou VPS en façade d'une machine à la maison / d'une instance
distante (le serveur n'expose alors jamais son IP). MCManager génère le bloc `stream` (module `libnginx-mod-stream`).
Limite de nginx : un **port public par serveur** (il ne lit pas l'adresse du handshake Minecraft pour router par domaine).

## Interface web derrière un nom de domaine

Renseignez « Domaine de l'interface » : vous obtenez un `server { … }` nginx avec **WebSocket** (indispensable à la
console), envois jusqu'à 512 Mo et les commandes `certbot` pour HTTPS, **ou** un Caddyfile de 4 lignes qui gère HTTPS
tout seul. Puis, dans `/etc/mcmanager/server.env` :

```
MCMANAGER_TRUST_PROXY=1
MCMANAGER_COOKIE_SECURE=1
```

Sans cela, le blocage anti-force-brute verrait toutes les connexions comme venant du proxy, et le cookie de session ne
serait pas marqué `Secure`.

## Sécurité

Tout ce qui est inséré dans une configuration (domaine, hôte, port) est validé strictement (lettres minuscules,
chiffres, tirets, points) : un « domaine » ne peut pas injecter de directive nginx. Un domaine ne peut servir qu'un
seul serveur. L'IP publique n'est détectée (via api.ipify.org) que si vous cliquez sur « Détecter ».
