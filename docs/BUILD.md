# Compiler et empaqueter MCManager

## Prérequis communs
- Rust 1.75+ (`rustc`, `cargo`) — via [rustup](https://rustup.rs) de préférence.
- Sur Linux : `pkg-config`, headers OpenSSL (`libssl-dev` sur Debian/Ubuntu).

```bash
cargo build --release
# binaire : target/release/mcmanager
```

## `build.sh`

`./build.sh [all|linux|windows|nix|installer]` — voir le script pour le
détail. Tous les artefacts sont produits dans `./dist/`.

### Linux → `.deb`
Nécessite `dpkg-deb` (paquet `dpkg-dev`, présent par défaut sur
Debian/Ubuntu). Le script construit une arborescence `usr/bin`,
`usr/share/mcmanager/web`, un service systemd optionnel
(`usr/lib/systemd/user/mcmanager.service`) puis appelle `dpkg-deb --build`.

### NixOS / GLF OS
`flake.nix` (à la **racine** du dépôt — important pour que
`nix run github:yo-le-zz/MCmanager` fonctionne sans préciser de sous-dossier)
fournit :
- un paquet (`nix build`/`nix run`),
- un module NixOS (`services.mcmanager.enable = true;`).

`packaging/nix/default.nix` est fourni pour les installations Nix sans flakes.
Si l'environnement de build ne dispose pas de `nix`, `build.sh nix` se
contente de regrouper ces fichiers (ils sont ensuite construits directement
sur une machine NixOS/GLF OS, ce qui est l'usage normal d'un flake).

### Windows
Deux façons d'obtenir un binaire Windows :

1. **Cross-compilation depuis Linux** (fonctionne, mais nécessite `rustup`) :
   ```bash
   sudo apt install mingw-w64
   rustup target add x86_64-pc-windows-gnu   # nécessite rustup, PAS le rustc d'apt/dnf
   ./build.sh windows
   ```
   ⚠️ **Important** : le `rustc`/`cargo` installé via le gestionnaire de
   paquets de votre distribution (`apt install cargo`, `dnf install cargo`...)
   ne contient QUE la bibliothèque standard pour votre propre plateforme. La
   std Windows n'est distribuée que via [rustup](https://rustup.rs) — sans
   lui, `./build.sh windows` échoue proprement et vous l'indique clairement
   (`rustup target add` échoue) plutôt que de planter en silence.
   Une fois `rustup` en place, la cross-compilation fonctionne normalement :
   le projet utilise `rustls` (pas OpenSSL) côté client HTTP pour éviter les
   soucis de liaison avec des libs C lors du cross-build.

2. **Build natif sur Windows ou CI** (recommandé pour une release officielle) :
   ```powershell
   cargo build --release
   ```
   ou via GitHub Actions avec `runs-on: windows-latest`.

### Installateur Windows (.exe façon "assistant d'installation")
`build.sh installer` génère `dist/mcmanager.iss`, un script
[Inno Setup](https://jrsoftware.org/isinfo.php) prêt à l'emploi. Pour obtenir
le `.exe` d'installation :
```powershell
ISCC.exe dist\mcmanager.iss
```
Un vrai `.msi` peut aussi être produit avec [WiX Toolset](https://wixtoolset.org/)
si vous préférez ce format — non fourni par défaut ici car il nécessite un
environnement Windows/WiX pour être généré fiablement.

## CI recommandée (GitHub Actions)

Une matrice `ubuntu-latest` / `windows-latest` avec `cargo build --release`
sur chaque OS, plus une étape Nix sur `ubuntu-latest` avec
`cachix/install-nix-action`, reproduit fidèlement tout ce que `build.sh` fait
localement, sans les limitations de cross-compilation.
