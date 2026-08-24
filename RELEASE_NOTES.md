# MCManager v1.0.5 — Correctifs Java critiques, assistant IA agissant, markdown

Cette version corrige deux bugs bloquants remontés sur la v1.0.4 (Java mal
pris en compte, paramètres qui semblaient ne pas s'enregistrer), et ajoute
un assistant IA capable d'agir directement plutôt que de seulement suggérer.

## 🔴 Corrections critiques

### Le Java configuré n'était jamais utilisé sur un serveur existant
Il n'existait qu'un réglage Java **global**, appliqué uniquement aux
serveurs créés *après* l'avoir changé. Un serveur déjà existant restait
bloqué sur le Java par défaut du système — d'où des erreurs comme
`Invalid maximum heap size` avec `-Xmx4096M`, alors que la même commande
lancée à la main avec le bon Java fonctionnait très bien.

**Corrigé** : un champ Java **par serveur** dans Paramètres, avec un
bouton **"🧪 Tester"** qui lance réellement `<ce java> -Xmx<valeur>
-version` et affiche le résultat exact — pour repérer un mauvais Java
avant de démarrer le serveur, pas après un crash en pleine partie.

### Les paramètres de serveur semblaient ne jamais s'enregistrer
Après avoir cliqué "Enregistrer", l'interface republiait les anciennes
valeurs — le serveur avait pourtant bien pris en compte le changement,
mais le cache local du navigateur n'était rafraîchi qu'en changeant de
page, jamais juste après la sauvegarde. **Corrigé** : la réponse de
sauvegarde met maintenant à jour ce cache immédiatement.

## 🤖 L'assistant IA peut maintenant agir

Jusqu'ici l'assistant ne faisait que suggérer en texte. Il dispose
maintenant d'outils réels (branchés sur Anthropic et Ollama) :
- **Installer un mod/plugin** directement depuis la conversation.
- **Lister ce qui est déjà installé**, pour éviter de proposer un doublon.

Boucle d'outils bornée à 4 étapes, pour qu'un modèle confus ne puisse pas
enchaîner des installations en boucle.

## ✨ Autres ajouts

- **Markdown dans les réponses IA** : gras, listes, blocs de code, liens
  et titres sont maintenant rendus proprement.
- **"👁 Suivre" un mod/plugin déjà installé** : l'identifie via Modrinth
  (par empreinte de fichier) et l'ajoute à la liste "gérée" sans avoir à
  connaître son slug/ID.
- **Traductions manquantes comblées** : titres de toutes les pages et
  plusieurs boutons "Enregistrer"/"Supprimer" traduits — couverture encore
  partielle, voir ci-dessous.

## 📥 Téléchargements

| Plateforme | Fichier |
|---|---|
| Linux (Debian/Ubuntu) | `mcmanager_1.0.5_amd64.deb` (inclut `mcmanager` et `mcmanager-headless`) |
| Linux (portable) | `mcmanager-1.0.5-linux-x86_64.tar.gz` |
| NixOS / GLF OS | `nix run github:yo-le-zz/MCmanager` |
| Windows (portable) | `mcmanager-1.0.5-windows-x86_64.zip` |
| Windows (installeur) | `mcmanager-1.0.5-setup.exe` |

**Changelog complet :** voir [CHANGELOG.md](./CHANGELOG.md)

## ⏭ Pas encore fait — reporté, en toute transparence

Cette liste vient d'une longue demande de fonctionnalités reçue pour cette
version ; certaines sont de vrais chantiers à part entière et n'ont pas pu
être faites correctement dans le temps disponible pour cette release
plutôt que de les livrer bâclées :

- **RCON + suivi TPS**
- **Contrôle à distance du dashboard pour les instances headless, avec
  échange de clés RSA** (chantier de sécurité à part entière — mérite
  d'être fait posément, pas dans la foulée d'autre chose)
- **Site web mcmanager.pages.dev** avec animations et commandes
  d'installation Windows/Linux/NixOS
- **Enregistrement dans le menu Démarrer Windows et le menu applications
  Linux**, avec désinstallation propre
- **Coloration syntaxique complète des fichiers** (YAML, etc.) dans
  l'éditeur de fichiers
- **Couverture i18n à 100 %** (le contenu long des cartes de paramètres et
  certains labels de formulaire restent en français)
