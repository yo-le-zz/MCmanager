# Bot Discord

> English version: [DISCORD.en.md](DISCORD.en.md)

Notifications, commandes `/mc` et, **pour chaque serveur Minecraft, une catégorie Discord dédiée** avec :

| Salon | Rôle |
|---|---|
| `status` | notifications de CE serveur (démarrage, prêt, arrêt, crash, sauvegarde…) |
| `console` | sortie de la console **en direct** ; les rôles autorisés y **tapent des commandes** |
| `chat` | chat du jeu **⇄** Discord (dans les deux sens) |

Réglages : **Paramètres → 🤖 Bot Discord**. Le bot se connecte *vers* Discord : aucun port à ouvrir, aucun domaine.
Il tourne dans le processus MCManager (`mcmanager` ou `mcmanager-server`) et reste inactif tant qu'il n'est pas activé.

## Mise en place

1. <https://discord.com/developers/applications> → **New Application** → **Bot** → copiez le **token**.
   Toujours dans *Bot* → *Privileged Gateway Intents* : activez **Message Content Intent** (nécessaire pour la console et le chat
   dans Discord ; sans lui, notifications et commandes `/mc` marchent quand même).
2. Dans MCManager, collez le token et cliquez sur **🔍 Détecter** : le lien d'**invitation** apparaît (avec toutes les
   permissions nécessaires : créer salons et droits, écrire, réagir). Invitez le bot, cliquez de nouveau sur *Détecter* :
   votre serveur Discord, ses **rôles** (cases à cocher colorées) et ses **salons** (menu) sont chargés.
3. **Qui peut faire quoi ?** Cochez : les rôles *opérateurs* (démarrer/arrêter, commandes console), les rôles qui *lisent* la
   console, ceux qui ont accès au *chat*. **Par défaut personne ne peut piloter** : choisissez au moins un rôle opérateur.
4. **Salons par serveur Minecraft** : pour chaque serveur, **✨ Créer les salons**. Réglez la console en direct, le pont de chat
   et, au besoin, des rôles supplémentaires autorisés à taper des commandes *sur ce serveur seulement*.
5. Cochez **Activer**, enregistrez, **📨 Envoyer un test**.

## Console dans Discord

- Les lignes du serveur arrivent par lots de 2 s dans `console`, en blocs de code (couleurs retirées). Au plus 3 messages par
  lot : en cas de flot de logs, un résumé « N lignes ignorées » évite de saturer Discord.
- Un message écrit dans `console` par un rôle autorisé est envoyé comme **commande** (le `/` initial est facultatif) ; réaction
  ✅ si envoyée, ❌ si impossible (serveur éteint), ⛔ si le rôle n'a pas le droit, ⚠️ si le message contient plusieurs lignes
  (une commande par message).
- Qui peut taper : les **opérateurs** (rôles ou utilisateurs) + les rôles ajoutés pour ce serveur. Les autres rôles « lecteurs »
  voient la console mais ne peuvent pas écrire — Discord le leur interdit **et** MCManager le revérifie à chaque message.

## Chat Minecraft ⇄ Discord

- Jeu → Discord : `**Alex** » bonjour` dans `chat`. Les connexions/déconnexions restent dans `status`.
- Discord → jeu : affiché à tous les joueurs sous la forme `[Discord] <Pseudo> message` (`tellraw`). Le pseudo est celui du
  serveur Discord (surnom). Texte nettoyé (retours à la ligne, codes `§`), 240 caractères maximum, impossible d'injecter
  une autre commande.
- Aucun message du bot ne peut pinguer `@everyone`, `@here` ou un rôle.

## Commandes `/mc`

Déclarées en anglais, **affichées en français** aux clients Discord en français ; les réponses suivent la langue de l'utilisateur.

| Anglais | Français | Qui | Effet |
|---|---|---|---|
| `/mc list` | `/mc liste` | tous | serveurs et état 🟢/⚫ (instances distantes comprises) |
| `/mc status server` | `/mc statut serveur` | tous | en ligne ? joueurs / max |
| `/mc players server` | `/mc joueurs serveur` | tous | joueurs connectés |
| `/mc start server` | `/mc demarrer serveur` | **opérateurs** | démarre |
| `/mc stop server` | `/mc arreter serveur` | **opérateurs** | arrêt propre |
| `/mc restart server` | `/mc redemarrer serveur` | **opérateurs** | redémarre |
| `/mc command server text` | `/mc commande serveur texte` | **opérateurs** | commande console |

## Langue

*Langue du bot* (français / anglais) : notifications et messages dans les salons. Les réponses aux commandes `/mc` suivent
la langue du client Discord de la personne (français → français, autre → anglais).

## Sécurité

- **Refus par défaut** : sans opérateur configuré, personne ne démarre, n'arrête ni n'envoie de commande.
- Salons créés **invisibles pour `@everyone`**, visibles seulement des rôles choisis. Les droits d'écriture sont **revérifiés**
  par MCManager à chaque message (ne jamais se fier aux seuls droits de salon).
- Token et webhook **chiffrés sur disque**, jamais renvoyés à l'interface. Commandes limitées à *votre* serveur Discord.
- Chaque action de pilotage est journalisée avec l'identifiant Discord de la personne.
- Un opérateur peut envoyer **n'importe quelle** commande (`op`, `stop`…) : ne l'accordez qu'à des personnes de confiance.
- Le bot ne demande **jamais** la permission Administrateur.

## Dépannage

- **« Console et chat indisponibles… Message Content Intent »** : activez l'intention privilégiée dans le portail développeur
  (étape 1), puis enregistrez à nouveau dans MCManager.
- **Création des salons : 403 / « Missing Permissions »** : invitez le bot avec le lien fourni (il inclut *Gérer les salons* et
  *Gérer les rôles*), et vérifiez que **le rôle du bot est au-dessus** des rôles qu'il doit gérer (Paramètres du serveur → Rôles).
- **« token du bot refusé (4004) »** : token recopié avec un espace, ou régénéré depuis.
- **`/mc` n'apparaît pas** : bot invité sans `applications.commands` → ré-invitez-le avec le lien fourni.
- Les évènements d'un serveur sont notifiés par l'instance **sur laquelle il tourne** ; pour un serveur distant, configurez aussi
  le bot (ou un webhook) sur cette instance. La console et le chat d'un serveur distant, eux, sont relayés par l'instance où
  le bot est configuré.
