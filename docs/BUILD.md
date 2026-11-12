# Compiler et empaqueter MCManager

## Prérequis communs
- Rust 1.75+ (`rustc`, `cargo`) — via [rustup](https://rustup.rs) de préférence.
- Sur Linux : `pkg-config`, headers OpenSSL (`libssl-dev` sur Debian/Ubuntu).

```bash
cargo build --release
# binaire : target/release/mcmanager
```

## `build.sh`

`./build.sh [all|linux|linux-arm64|windows|windows-arm64|macos|installer]` — voir le script pour le
détail. Tous les artefacts sont produits dans `./dist/`.

### Linux → `.deb`
Nécessite `dpkg-deb` (paquet `dpkg-dev`, présent par défaut sur
Debian/Ubuntu). Le script construit une arborescence `usr/bin`,
`usr/share/mcmanager/web`, un service systemd optionnel
(`usr/lib/systemd/user/mcmanager.service`) puis appelle `dpkg-deb --build`.

### Compatibilité des binaires Linux (glibc)
Un binaire Rust est lié à la glibc de la machine qui le compile : compilé sur une distribution récente, il refuse de démarrer
sur une plus ancienne (`version 'GLIBC_2.38' not found`). Le workflow GitHub compile donc sur **Ubuntu 22.04 (glibc 2.35)** ;
les binaires tournent sur Debian 12, Ubuntu 22.04+, Raspberry Pi OS Bookworm, RHEL/Rocky/Alma 9+, Fedora et openSUSE récents.
Pour des systèmes plus anciens (Debian 11, Ubuntu 20.04), utilisez l'image Docker ou compilez sur place.

### macOS
`./build.sh macos` — **à lancer sur un Mac** (pas de cross-compilation depuis Linux : le SDK Apple n'est pas redistribuable).
Produit `dist/MCManager.app`, `mcmanager-<version>-macos-<arch>.dmg` et `.tar.gz` pour l'architecture du Mac (`arm64` ou `x86_64`),
avec une cible minimale macOS 11. L'assemblage est fait par `scripts/make-macos-app.sh` ; la signature est « ad hoc »,
sans notarisation (voir [MACOS.md](MACOS.md)). Le workflow GitHub compile les deux architectures sur des runners natifs.

### Docker
`docker build -t mcmanager .` construit l'image du mode serveur (Dockerfile à la racine, build multi-étapes,
Java 21 + 17 dans l'image finale). `docker compose up -d` la lance avec un volume de données.

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

Deux workflows sont fournis dans `.github/workflows/` :
- `ci.yml` (chaque push et pull request) : `cargo test --lib` sur Ubuntu 22.04 et macOS, compilation Windows, `cargo check` avec
  Rust 1.75 (les versions de dépendances sont épinglées dans `Cargo.toml`), tests de l'interface (`node scripts/test-ui.js`),
  syntaxe des scripts et cohérence de `scripts/install.*` avec `website/install.*`.
- `release.yml` (tag `v*.*.*`) : compilation **native** sur chaque architecture — Linux x86_64 et ARM64 (`.deb`, `.rpm`, `.tar.gz`),
  macOS arm64 et x86_64 (`.dmg`, `.tar.gz`), Windows x86_64 et ARM64 (`.exe`, `.zip`), image Docker multi-architecture sur ghcr.io —
  puis publication de la release avec un fichier `SHA256SUMS` (`sha256sum -c SHA256SUMS`).
