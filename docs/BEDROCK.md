# Minecraft Bedrock Edition (depuis la 1.1.3)

MCManager gère des serveurs **Bedrock** (mobile, consoles, Windows 10/11) comme les serveurs Java : console, sauvegardes,
Discord, statistiques, contrôle à distance. Les joueurs se connectent en **UDP** (port **19132** par défaut).

## Quatre types de serveur

| Type | Ce que c'est | Plateformes | Extensions |
|---|---|---|---|
| **Officiel** (`bedrock`) | Serveur natif de Mojang (« Bedrock Dedicated Server ») | **x86_64 Linux et Windows seulement** | Add-ons `.mcpack` / `.mcaddon`, packs de ressources, mondes `.mcworld` |
| **Officiel Preview** (`bedrockpreview`) | Versions bêta de Mojang | idem | idem (utilisez un monde séparé : une bêta peut abîmer un monde) |
| **PocketMine-MP** (`pocketmine`) | Serveur communautaire en PHP | y compris **ARM / Raspberry Pi** | Plugins `.phar` |
| **Nukkit** (`nukkit`) | Serveur communautaire en Java | y compris **ARM / Raspberry Pi** | Plugins `.jar` |

Le serveur officiel n'existe pas pour ARM ni pour macOS : MCManager l'indique clairement à la création. Sur un Raspberry Pi,
choisissez PocketMine-MP ou Nukkit. PocketMine a besoin de PHP 8 avec l'extension `pmmpthread` : si le vôtre ne convient pas,
MCManager télécharge les binaires PHP officiels. Nukkit demande Java 17 ou plus.

## Créer un serveur

Assistant : **Serveurs → Nouveau serveur → Édition : Bedrock Edition**, puis le type. Le port passe à 19132 (UDP) et le réglage
de mémoire disparaît pour l'officiel (programme natif) et PocketMine. En ligne de commande :
`mcmanager cli create --name Famille --loader bedrock --version 1.21.50.07` (port 19132 par défaut).

Les versions viennent de l'API de téléchargement de Mojang et de la liste communautaire `Bedrock-OSS/BDS-Versions`.
Si elles sont injoignables (hors ligne), la liste de versions l'indique au lieu de rester vide.

## Ce que MCManager fait pour Bedrock

- **Packs & mondes** (serveur officiel) : installation de `.mcpack`, `.mcaddon` et de packs imbriqués ; lecture des manifestes
  (JSON avec commentaires ou virgules finales) ; **activation par monde** avec leurs dépendances ; import, export et
  changement du monde actif (`.mcworld`). Un pack ne fonctionne qu'une fois actif dans le monde ; redémarrez ensuite le serveur.
- **Liste d'autorisation et permissions** (`allowlist.json`, `permissions.json`) : pseudos Xbox (gamertags), niveaux visiteur /
  membre / opérateur. Les changements s'appliquent **immédiatement** si le serveur tourne.
- **Propriétés** : un catalogue de réglages (types, bornes, valeurs permises) vérifié avant l'enregistrement.
- **Sauvegarde à chaud** : `save hold` → `save query` → copie des fichiers tronqués aux longueurs annoncées → `save resume`.
  Le monde n'est jamais copié en cours d'écriture ; le serveur reste allumé.
- **État** : le ping utilise le protocole RakNet (UDP). Les connexions sont lues dans la console (`Player connected: … xuid: …`).
- **Changement de version** : sauvegarde de sécurité d'abord ; `server.properties`, `allowlist.json`, `permissions.json` et les mondes sont conservés.
- **Veille** (« serveur dynamique ») : MCManager écoute en UDP sur le port du serveur, répond au ping de la liste de serveurs, et
  démarre le serveur quand un joueur tente de se connecter (le joueur réessaie une fois le serveur prêt). *Testé avec des paquets
  simulés, pas avec un vrai client Bedrock.*
- **Discord** : `/mc join` donne l'adresse et le port (pas de SRV pour Bedrock).
- **Domaines** : MCManager génère un bloc nginx `stream … udp`, pas d'enregistrement SRV.
- **Joueurs Bedrock sur un serveur Java** : presets **Geyser** et **Floodgate**.

## Réseau

Ouvrez le port **UDP** 19132 (et pas TCP) sur votre routeur, ou créez un tunnel UDP dans playit.gg (voir [PLAYIT.md](PLAYIT.md)).

## Limites connues

- Les téléchargements réels (Mojang, Bedrock-OSS, PocketMine, binaires PHP, Nukkit) n'ont pas été exécutés avant la publication ;
  le format de la liste Bedrock-OSS et le nom des binaires PHP sont supposés et analysés de façon tolérante.
- Le serveur Bedrock officiel a été testé de bout en bout avec un **faux programme** (démarrage, sauvegarde à chaud, joueur, arrêt), pas avec le vrai.
