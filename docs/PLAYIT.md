# Tutoriel : jouer avec vos amis via playit.gg

playit.gg permet d'exposer votre serveur Minecraft sur Internet **sans ouvrir
de port sur votre routeur** (pas de redirection NAT à configurer).

## Étapes (aussi disponibles dans l'onglet "Réseau" de MCManager)

1. Ouvrez l'onglet **Réseau (playit.gg)** dans MCManager.
2. Cliquez sur **Installer / mettre à jour l'agent** — MCManager télécharge le
   binaire officiel `playit-agent` depuis GitHub pour votre plateforme.
3. Cliquez sur **Démarrer**. La console affiche un lien du type :
   ```
   Visit https://playit.gg/claim/XXXXXXXX to setup the agent
   ```
4. Ouvrez ce lien, connectez-vous (ou créez un compte gratuit), et validez
   pour "réclamer" cet agent — il apparaîtra ensuite dans votre tableau de
   bord playit.gg.
5. Sur le site playit.gg, créez un **tunnel** de type **Minecraft Java**
   pointant vers le port local de votre serveur (visible dans l'onglet
   Serveurs de MCManager, `25565` par défaut).
6. playit.gg vous fournit une adresse publique, par exemple :
   ```
   mon-serveur.gl.joinmc.link
   ```
   Partagez cette adresse à vos amis — c'est celle qu'ils utiliseront pour se
   connecter dans le client Minecraft (pas besoin de connaître votre IP).

## Bon à savoir

- L'agent doit rester **démarré** tant que vous voulez que le serveur soit
  joignable depuis l'extérieur.
- Le plan gratuit de playit.gg est largement suffisant pour un serveur entre
  amis.
- Vous pouvez arrêter/redémarrer l'agent à tout moment depuis MCManager sans
  affecter le serveur Minecraft lui-même (ce sont deux processus séparés).
- Alternative : ouvrir manuellement le port du serveur sur votre routeur
  (redirection de port vers le port du serveur, généralement 25565) et
  partager votre IP publique — plus technique mais pas de dépendance à un
  service tiers.
