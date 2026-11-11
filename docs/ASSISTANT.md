# Assistant

L'assistant répond en direct (streaming) et peut agir sur le serveur sélectionné. Il fonctionne avec les cinq
fournisseurs : Anthropic, OpenAI, Gemini, Ollama et OmniRoute. La clé d'API est chiffrée sur disque (voir `src/ai.rs`).

## Mode des outils (Assistant -> Fournisseur)
| Mode | Effet |
|---|---|
| `off` | aucun outil : réponses en texte |
| `read` | lecture seule |
| `ask` (défaut) | lecture + écriture ; **chaque écriture attend votre accord** (120 s, sans réponse = refus) |
| `auto` | lecture + écriture sans confirmation |

## Outils
- **Lecture** : `get_server_status`, `read_console_logs`, `list_installed_addons`, `read_server_properties` (mots de passe masqués),
  `list_backups`, `search_addons` (Modrinth, filtré sur le loader et la version), `web_search`, `web_fetch`.
- **Écriture** : `install_addon`, `remove_addon`, `set_server_property`, `send_console_command`, `create_backup`,
  `start_server`, `stop_server`, `restart_server`.
- Un serveur d'une autre machine (jumelée) n'est pas pilotable par l'assistant. Sans serveur sélectionné, seuls les outils web sont proposés.
- Chaque résultat est tronqué à 6000 caractères. Un tour d'agent compte au plus 8 appels successifs au modèle.
- Si un modèle refuse les outils, la requête est refaite sans eux et la réponse l'indique.

## Sécurité
- Le contenu des journaux, des pages web et des fichiers est présenté au modèle comme une **donnée non fiable**.
- `web_fetch` : http/https seulement, ports 80/443/8080/8443, pas d'identifiants dans l'URL, noms locaux refusés, DNS résolu puis
  **épinglé**, toute adresse non publique refusée (127.0.0.1, 169.254.x.x, réseaux privés, IPv6 locale...), redirections revérifiées (4 au plus),
  corps limité à 512 Ko, types texte / json / xml.
- `set_server_property` refuse les clés secrètes ; `remove_addon` n'efface que des fichiers listés ; les identifiants sont validés.
- Les écritures (acceptées, refusées ou échouées) sont inscrites dans le journal d'audit (`kind: "ai"`), sans valeur ni secret
  (seul le premier mot d'une commande est consigné).
- Clé Gemini envoyée dans l'en-tête `x-goog-api-key`, jamais dans l'URL.

## Protocole
Voir `docs/API.md` (section Assistant).
