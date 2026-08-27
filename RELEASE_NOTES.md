# MCManager v1.0.7 — Onglet Contrôle à distance dans l'interface web

Petite version corrective de clarté après la v1.0.6 : le contrôle à
distance RSA introduit en v1.0.6 n'avait, à sa sortie, aucune interface
navigable — seulement des commandes CLI côté `mcmanager-headless`. Si tu
avais installé la v1.0.6 et cherché l'onglet dans l'app web, c'est
normal qu'il n'y était pas encore : le voilà.

## 🖧 Onglet "Contrôle à distance"

Dans l'interface web (`mcmanager`, pas seulement `mcmanager-headless`) :

- Jumeler une instance distante (code à usage unique + vérification
  d'empreinte, comme en CLI).
- Lister/démarrer/arrêter/redémarrer les serveurs d'une instance
  distante déjà jumelée.
- **Envoyer un serveur local vers l'instance distante** : copie le
  dossier complet et l'enregistre là-bas comme nouveau serveur.

Le navigateur ne fait aucune cryptographie lui-même : il parle en HTTP
normal à son propre backend MCManager, qui gère le chiffrement RSA+AES
vers l'instance distante en réutilisant le client déjà écrit (et testé
par un test d'intégration bout-en-bout) pour `mcmanager-headless`.

## 🌗 Site web

Bascule clair/sombre (sombre par défaut), interrupteur en haut à droite,
préférence mémorisée localement.

## 📥 Téléchargements

| Plateforme | Fichier |
|---|---|
| Linux (Debian/Ubuntu) | `mcmanager_1.0.7_amd64.deb` |
| Linux (portable) | `mcmanager-1.0.7-linux-x86_64.tar.gz` |
| NixOS / GLF OS | `nix run github:yo-le-zz/MCmanager` |
| Windows (portable) | `mcmanager-1.0.7-windows-x86_64.zip` |
| Windows (installeur) | `mcmanager-1.0.7-setup.exe` |

**Changelog complet :** voir [CHANGELOG.md](./CHANGELOG.md)
