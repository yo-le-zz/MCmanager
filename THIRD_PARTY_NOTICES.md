# Éléments tiers embarqués dans l'interface web

Tout est **embarqué** dans `web/assets` : l'interface ne charge rien depuis Internet (elle fonctionne hors ligne et sur un réseau local).

| Élément | Fichier | Licence |
|---|---|---|
| [Lucide](https://lucide.dev) — icônes (sélection de 96 icônes assemblées en sprite SVG) | `web/assets/icons.svg` | ISC (basé en partie sur Feather, MIT) |
| [Inter](https://rsms.me/inter/) — police de l'interface | `web/assets/fonts/inter.woff2` | SIL Open Font License 1.1 |
| [JetBrains Mono](https://www.jetbrains.com/lp/mono/) — police de la console et du code | `web/assets/fonts/jetbrains-mono.woff2` | SIL Open Font License 1.1 |
| [Natural Earth](https://www.naturalearthdata.com) (via le paquet `world-atlas`) — contours des terres, transformés en grilles de points | `web/assets/world-dots*.json` | Domaine public (Natural Earth) / ISC (world-atlas) |

Le fond animé en points et la carte du monde sont écrits pour MCManager (`web/ui.js`, `web/worldmap.js`), sans bibliothèque.
Les ressources se régénèrent avec `scripts/build-ui-assets.mjs` (Node.js).
