# Migration des données (depuis la 1.1.3)

Au premier démarrage d'une version 1.1.3 sur des données plus anciennes, MCManager les met à niveau **sans rien perdre** :

1. Un fichier `schema.json` mémorise la version du format des données (absent = données d'avant la 1.1.3, schéma 1 ; la 1.1.3 est le schéma 2).
2. **Avant toute modification**, les fichiers de configuration sont copiés dans `migration-backups/v<schéma>-<date>/`.
3. `servers.json` est lu **serveur par serveur** : une entrée illisible (éditée à la main, type inconnu venant d'une version plus récente)
   est mise de côté dans `servers.rejected.json` ; les autres serveurs se chargent normalement. Avant la 1.1.3, une seule entrée
   invalide faisait disparaître toute la liste et la sauvegarde suivante écrasait le fichier.
4. Les champs apparus depuis les versions antérieures reçoivent leur valeur par défaut (les serveurs Bedrock prennent le port 19132).
5. La migration est **idempotente** : la relancer ne change rien.

État courant : `GET /api/migration`. Pour revenir en arrière, arrêtez MCManager et recopiez le contenu de `migration-backups/`.
