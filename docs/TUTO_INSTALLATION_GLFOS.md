# Installer MCManager sur GLF OS / NixOS — tuto simple

Pas besoin de savoir coder, une seule commande suffit.

## Installer et lancer

Ouvre un terminal et tape :

```bash
nix run github:yo-le-zz/MCmanager
```

La première fois, ça va télécharger et compiler des trucs (ça peut prendre
quelques minutes, c'est normal). Une fois que c'est fini, tu verras un
message du genre :

```
MCManager v1.0.1 demarre sur http://127.0.0.1:7777
```

Ton navigateur par défaut s'ouvre automatiquement sur l'interface. Sinon,
ouvre-le toi-même et va sur `http://127.0.0.1:7777`.

Tu peux créer un serveur Minecraft directement depuis là (bouton "Nouveau
serveur"). Pour arrêter MCManager : retourne dans le terminal et fais
`Ctrl + C`.

## Mettre à jour

Relance simplement la même commande :

```bash
nix run github:yo-le-zz/MCmanager
```

Nix va automatiquement récupérer la dernière version du projet sur GitHub et
la recompiler si besoin — pas besoin de "désinstaller" quoi que ce soit.

---

## Option : le lancer automatiquement au démarrage (plus avancé)

Si tu veux que MCManager tourne en permanence sans avoir à taper la commande
à chaque fois, ajoute ceci dans ta config NixOS (`/etc/nixos/flake.nix`) :

```nix
inputs.mcmanager.url = "github:yo-le-zz/MCmanager";
# ...
services.mcmanager.enable = true;
```

Ce n'est pas obligatoire pour juste essayer MCManager.

---

## Un souci ?

- **"command not found: nix"** → Nix n'est pas installé sur ta machine.
  Étrange sur GLF OS normalement (Nix est inclus par défaut). Vérifie avec
  `which nix`.
- **"nix run" prend très longtemps** → c'est normal la première fois
  (compilation). Les fois suivantes seront beaucoup plus rapides.
- **Erreur "flake.nix not found" ou du genre** → assure-toi de taper
  exactement `nix run github:yo-le-zz/MCmanager` (sans rien ajouter après) ;
  versions antérieures à 1.0.1 de ce tuto pointaient vers un mauvais chemin,
  c'est corrigé depuis.
- **Ça affiche une erreur bizarre** → fais une capture d'écran du terminal et
  envoie-la à la personne qui t'a donné le projet.
