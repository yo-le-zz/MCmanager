# MCManager v1.0.3 — OmniRoute, et corrections de release/mise à jour importantes

Cette version ajoute **OmniRoute** comme fournisseur pour l'assistant IA, et
corrige trois bugs qui empêchaient les releases précédentes de fonctionner
correctement en pratique : le binaire Windows incomplet, la mise à jour
automatique cassée, et la création de release GitHub qui échouait.

## 🤖 Nouveau fournisseur IA — OmniRoute

[OmniRoute](https://omniroute.online/) ([GitHub](https://github.com/diegosouzapw/OmniRoute))
est une passerelle IA auto-hébergée, compatible OpenAI, qui donne accès à
300+ fournisseurs (Claude, GPT, Gemini, Kimi, DeepSeek...) derrière une
seule clé API. Elle s'ajoute aux quatre fournisseurs existants
(Anthropic, OpenAI, Gemini, Ollama) dans l'onglet Assistant IA :

- Champ d'URL dédié, par défaut `http://127.0.0.1:20128/v1` (le port local
  standard d'OmniRoute) — modifiable si vous faites tourner OmniRoute
  ailleurs.
- Détection automatique des modèles disponibles.
- Modèle par défaut `auto` (le routage intelligent zero-config d'OmniRoute).
- Clé API chiffrée sur disque (AES-256-GCM), comme pour les autres
  fournisseurs.

## 🐛 Corrections importantes

### Le binaire Windows était incomplet
Le job Windows du workflow de release compilait sans `--bins` et ne
copiait que `mcmanager.exe` — jamais `mcmanager-headless.exe`, ni les
dernières fonctionnalités si le tag ne pointait pas sur le bon commit.
L'installateur Inno Setup avait le même trou. **Corrigé** : les deux
binaires sont maintenant inclus partout (archive portable, installateur,
paquet .deb).

### La mise à jour automatique ne détectait jamais rien
Cause racine trouvée : le dépôt GitHub réel est `yo-le-zz/MCmanager`, mais
le code de vérification de mise à jour (et le user-agent HTTP, les liens
`nix run`, le README, la doc, les scripts de build) pointait vers
`yolezz/mcmanager` — **un dépôt différent qui n'existe pas**. Chaque
vérification échouait silencieusement (404), donc l'application ne
détectait jamais de nouvelle version, quel que soit le tag publié.
**Corrigé partout** dans le code et la documentation.

### La création de release GitHub échouait (403)
Le token `GITHUB_TOKEN` par défaut n'a que les droits de lecture sur le
dépôt tant que `permissions: contents: write` n'est pas déclaré
explicitement dans le workflow. **Ajouté** au niveau du workflow et du job
`publish` — les releases se créent maintenant normalement.

### Nettoyage annexe
- Mise à jour des actions GitHub (`checkout`, `upload-artifact`,
  `download-artifact`, `action-gh-release`) vers leurs dernières versions
  majeures (Node 24 natif), pour faire disparaître l'avertissement de
  dépréciation Node 20 dans les logs.
- Versions de paquet Nix resynchronisées avec la version réelle de
  l'application.

## 📥 Téléchargements

| Plateforme | Fichier |
|---|---|
| Linux (Debian/Ubuntu) | `mcmanager_1.0.3_amd64.deb` (inclut `mcmanager` et `mcmanager-headless`) |
| Linux (portable) | `mcmanager-1.0.3-linux-x86_64.tar.gz` |
| NixOS / GLF OS | `nix run github:yo-le-zz/MCmanager` |
| Windows (portable) | `mcmanager-1.0.3-windows-x86_64.zip` |
| Windows (installeur) | `mcmanager-1.0.3-setup.exe` |

**Changelog complet :** voir [CHANGELOG.md](./CHANGELOG.md)

> ⚠️ Si vous mettez à jour depuis une version antérieure à 1.0.2 ou 1.0.3
> et que la mise à jour automatique ne détecte toujours rien après cette
> release : vérifiez que le tag `v1.0.3` a bien été poussé **après** ces
> correctifs (`git push origin v1.0.3` sur le dernier commit), pas avant —
> un tag existant ne bouge pas tout seul.
