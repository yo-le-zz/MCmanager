# Bot Discord

Notifications dans un salon **et** pilotage des serveurs par commandes slash. Réglages : **Paramètres → 🤖 Bot Discord**.

Le bot se connecte *vers* Discord (connexion sortante) : **aucun port à ouvrir, aucun domaine**. Il fonctionne derrière
une box, sur un Raspberry Pi, dans Docker. Il tourne dans le processus MCManager (`mcmanager` ou `mcmanager-server`) ;
il reste inactif tant qu'il n'est pas activé.

## Mise en place (5 minutes)

1. <https://discord.com/developers/applications> → **New Application** → onglet **Bot** → **Reset Token** → copiez le token.
2. Onglet **OAuth2 → URL Generator** : portées `bot` + `applications.commands`, permissions « Voir les salons » et
   « Envoyer des messages » → ouvrez l'URL générée pour inviter le bot sur votre serveur Discord.
3. Dans Discord : *Paramètres → Avancé → Mode développeur*. Clic droit sur votre serveur / un salon / votre profil →
   **Copier l'identifiant**.
4. Dans MCManager : collez le token, l'identifiant du serveur Discord (guilde), celui du salon de notifications, **votre**
   identifiant utilisateur dans *Opérateurs*, cochez **Activer**, enregistrez. L'état passe à « Connecté » en quelques secondes.
5. **Envoyer un test** pour vérifier le salon.

> Pas envie d'un bot ? Collez simplement une **URL de webhook** (Discord → paramètres du salon → Intégrations → Webhooks) :
> les notifications fonctionnent, sans commandes.

## Commandes

| Commande | Qui | Effet |
|---|---|---|
| `/mc liste` | tous (ou rôles de lecture) | serveurs et état 🟢/⚫ (instances distantes comprises) |
| `/mc statut serveur` | idem | en ligne ? joueurs connectés / max |
| `/mc joueurs serveur` | idem | nombre de joueurs |
| `/mc demarrer serveur` | **opérateurs** | démarre le serveur |
| `/mc arreter serveur` | **opérateurs** | arrêt propre (sauvegarde du monde) |
| `/mc redemarrer serveur` | **opérateurs** | arrêt puis démarrage |
| `/mc commande serveur texte` | **opérateurs** | envoie une commande à la console (`say Bonjour`, `whitelist add Alex`…) |

Le nom du serveur s'auto-complète. Les serveurs d'une instance jumelée (onglet *Contrôle à distance*) se pilotent de la
même façon, la commande est relayée par le tunnel chiffré.

## Notifications

Démarrage · serveur prêt (« Done! ») · arrêt · crash · sauvegarde terminée · redémarrage programmé · arrêt automatique
(inactivité) · connexion/déconnexion d'un joueur (désactivé par défaut). Chaque type s'active séparément.

> Les évènements d'un serveur sont notifiés par l'instance **sur laquelle il tourne**. Pour recevoir ceux d'un serveur
> distant, configurez aussi le bot (ou un webhook) sur cette instance.

## Sécurité

- **Refus par défaut** : tant qu'aucun opérateur (utilisateur ou rôle) n'est configuré, personne ne peut démarrer, arrêter
  ni envoyer de commande. Les identifiants sont vérifiés (suite de chiffres) à l'enregistrement.
- Les commandes ne sont enregistrées que dans **votre** serveur Discord ; toute interaction venant d'un autre est refusée.
- Le token du bot et l'URL du webhook sont **chiffrés sur disque**, jamais renvoyés à l'interface (seulement « configuré »).
- Chaque action de pilotage est journalisée avec l'identifiant Discord de la personne.
- Pour restreindre qui *voit* les commandes, utilisez aussi *Paramètres du serveur → Intégrations* dans Discord, ou les
  « rôles autorisés en lecture » de MCManager.
- Un opérateur peut envoyer **n'importe quelle** commande console (y compris `op`, `stop`) : ne l'accordez qu'à des
  personnes de confiance.

## Dépannage

- **« token du bot refusé (4004) »** : token recopié avec un espace, ou régénéré depuis (l'ancien est invalide).
- **Connecté mais `/mc` n'apparaît pas** : le bot a été invité sans la portée `applications.commands` ; ré-invitez-le
  avec l'URL de l'étape 2. Les commandes de serveur (guilde) apparaissent instantanément.
- **Test : 403 « Missing Access »** : le bot n'a pas accès au salon (permissions du salon).
- **Rien ne se passe sur `/mc demarrer`** : « ⛔ réservée aux opérateurs » → ajoutez votre identifiant dans *Opérateurs*.
