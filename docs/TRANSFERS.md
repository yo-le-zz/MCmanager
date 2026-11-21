# Transferts entre machines, miroir, modèles (depuis la 1.1.3)

Prérequis : une machine distante **jumelée** (Contrôle à distance, voir [SERVER.md](SERVER.md)). Les transferts sont lancés depuis la carte
**Transferts** de cette vue, ou par l'API.

## Principes
- **En flux** : fichiers lus et écrits par morceaux de 2 Mo, jamais chargés en mémoire.
- **Reprenables** : après une coupure, relancer la même opération (bouton « Réessayer ») reprend à l'octet déjà reçu.
- **Vérifiés** : empreinte SHA-256 comparée de bout en bout avant toute action irréversible.
- **Sûrs** : un serveur n'est supprimé de la machine d'origine qu'après avoir été importé **et retrouvé** sur la destination, et seulement
  pour l'opération « déplacer ». « Copier » ne supprime rien.

## Opérations
| Opération | Effet |
|---|---|
| Copier / déplacer un serveur | envoie un serveur local vers une machine |
| Rapatrier | récupère un serveur d'une machine distante (copie ou déplacement) |
| Envoyer une sauvegarde | pousse une sauvegarde vers une machine |
| Récupérer une sauvegarde | tire une sauvegarde reçue d'une machine |
| **Miroir automatique** | chaque nouvelle sauvegarde d'un serveur est envoyée à une machine choisie |
| Sauvegardes reçues | liste et suppression des sauvegardes arrivées d'autres machines |
| **Mettre à jour une machine distante** | bouton « Update » : vérifie et installe la dernière version de MCManager sur cette machine |

## Modèles de serveur
Bouton **Modèle** sur la carte d'un serveur : enregistre ses réglages, `server.properties`, plugins/mods et leur configuration
(jamais `logs/`, `crash-reports/`, `cache/` ; le monde seulement si vous le demandez). **Modèles de serveur** : créer un serveur depuis
un modèle ne télécharge rien (le modèle contient déjà le jar). Stockage : `<données>/templates/<id>/`.

## Non exécuté avant la publication
La mise à jour d'une machine distante réelle (`update_apply`) n'a pas été exécutée ; les transferts ont été testés entre deux
processus réels et deux machines en mémoire.
