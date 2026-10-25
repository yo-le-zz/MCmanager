#!/usr/bin/env bash
# ==============================================================================
#  MCManager - build.sh
#  Compile et empaquette MCManager pour Linux (.deb)
#  et Windows (binaire + installeur Inno Setup), puis regroupe tous les
#  artefacts dans ./dist/.
#
#  Usage:
#     ./build.sh                # build tout ce qui est possible sur cette machine
#     ./build.sh linux          # uniquement le binaire + .deb Linux
#     ./build.sh windows        # uniquement le cross-build Windows
#     ./build.sh installer      # (re)génère l'installateur/script d'installation
# ==============================================================================
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/version = "(.*)"/\1/')"
APP_NAME="mcmanager"
DIST_DIR="$ROOT_DIR/dist"
TARGET_LINUX="x86_64-unknown-linux-gnu"
TARGET_WINDOWS="x86_64-pc-windows-gnu"

log()  { printf "\033[1;34m[build]\033[0m %s\n" "$*"; }
ok()   { printf "\033[1;32m[ ok  ]\033[0m %s\n" "$*"; }
warn() { printf "\033[1;33m[warn ]\033[0m %s\n" "$*"; }
err()  { printf "\033[1;31m[fail ]\033[0m %s\n" "$*"; }

mkdir -p "$DIST_DIR"

TASK="${1:-all}"

# ------------------------------------------------------------------------------
build_linux() {
  log "Compilation du binaire Linux (release)…"
  cargo build --release --bins
  local bin="target/release/${APP_NAME}"
  local bin_server="target/release/${APP_NAME}-server"
  if [ ! -f "$bin" ]; then
    err "binaire introuvable après compilation"; exit 1
  fi
  ok "binaire compilé: $bin"
  if [ ! -f "$bin_server" ]; then
    warn "binaire ${APP_NAME}-server introuvable, non inclus dans le paquet (verifiez la compilation)"
  else
    ok "binaire compilé: $bin_server"
  fi

  # ---- archive portable .tar.gz ----
  local portable_dir="$DIST_DIR/${APP_NAME}-${VERSION}-linux-x86_64"
  rm -rf "$portable_dir"
  mkdir -p "$portable_dir"
  cp "$bin" "$portable_dir/"
  [ -f "$bin_server" ] && cp "$bin_server" "$portable_dir/"
  cp -r web "$portable_dir/"
  cp README.md LICENSE CHANGELOG.md "$portable_dir/" 2>/dev/null || true
  (cd "$DIST_DIR" && tar czf "${APP_NAME}-${VERSION}-linux-x86_64.tar.gz" "$(basename "$portable_dir")")
  ok "archive portable: dist/${APP_NAME}-${VERSION}-linux-x86_64.tar.gz"

  build_deb "$bin" "$bin_server"
}

# ------------------------------------------------------------------------------
build_deb() {
  local bin="$1"
  local bin_server="${2:-}"
  if ! command -v dpkg-deb >/dev/null 2>&1; then
    warn "dpkg-deb introuvable, paquet .deb ignoré (installez dpkg-dev)."
    return
  fi
  log "Construction du paquet .deb…"
  local arch
  arch="$(dpkg --print-architecture 2>/dev/null || echo amd64)"
  local pkg_dir="$DIST_DIR/deb-pkg"
  rm -rf "$pkg_dir"
  mkdir -p "$pkg_dir/DEBIAN" \
           "$pkg_dir/usr/bin" \
           "$pkg_dir/usr/share/mcmanager" \
           "$pkg_dir/usr/lib/systemd/user" \
           "$pkg_dir/usr/lib/systemd/system" \
           "$pkg_dir/etc/mcmanager" \
           "$pkg_dir/usr/share/doc/mcmanager" \
           "$pkg_dir/usr/share/applications" \
           "$pkg_dir/usr/share/icons/hicolor/256x256/apps"

  cp "$bin" "$pkg_dir/usr/bin/mcmanager"
  chmod 755 "$pkg_dir/usr/bin/mcmanager"
  if [ -n "$bin_server" ] && [ -f "$bin_server" ]; then
    cp "$bin_server" "$pkg_dir/usr/bin/mcmanager-server"
    chmod 755 "$pkg_dir/usr/bin/mcmanager-server"
  fi
  cp -r web "$pkg_dir/usr/share/mcmanager/web"
  cp packaging/deb/mcmanager.service "$pkg_dir/usr/lib/systemd/user/mcmanager.service"
  # mcmanager-server.service is a SYSTEM unit (not user): it needs to
  # start at boot without any user session existing yet, unlike the web
  # GUI's user unit above which only makes sense once someone is logged in.
  cp packaging/deb/mcmanager-server.service "$pkg_dir/usr/lib/systemd/system/mcmanager-server.service"
  touch "$pkg_dir/etc/mcmanager/autostart.txt"
  cp packaging/deb/mcmanager.desktop "$pkg_dir/usr/share/applications/mcmanager.desktop"
  cp web/assets/icon-256.png "$pkg_dir/usr/share/icons/hicolor/256x256/apps/mcmanager.png"
  cp README.md LICENSE CHANGELOG.md "$pkg_dir/usr/share/doc/mcmanager/" 2>/dev/null || true

  sed -e "s/__VERSION__/${VERSION}/" -e "s/__ARCH__/${arch}/" \
    packaging/deb/control > "$pkg_dir/DEBIAN/control"
  cp packaging/deb/postinst "$pkg_dir/DEBIAN/postinst"
  chmod 755 "$pkg_dir/DEBIAN/postinst"

  dpkg-deb --build --root-owner-group "$pkg_dir" "$DIST_DIR/${APP_NAME}_${VERSION}_${arch}.deb" >/dev/null
  ok "paquet Debian: dist/${APP_NAME}_${VERSION}_${arch}.deb"
  rm -rf "$pkg_dir"
}

# ------------------------------------------------------------------------------
build_windows() {
  log "Tentative de cross-compilation Windows (${TARGET_WINDOWS})…"

  # IMPORTANT : le rustc fourni par les gestionnaires de paquets Linux (apt,
  # dnf, pacman...) n'embarque QUE la std pour votre plateforme native. La std
  # Windows n'est distribuée que via rustup. Sans rustup, cette étape ne peut
  # pas fonctionner, quel que soit le reste de la configuration.
  if ! command -v rustup >/dev/null 2>&1; then
    warn "rustup est introuvable. Le rustc de votre distribution (apt/dnf/pacman) ne suffit"
    warn "PAS pour cross-compiler vers Windows : il ne contient pas la bibliotheque standard"
    warn "Windows, distribuee uniquement via rustup (https://rustup.rs)."
    warn "Installez rustup puis relancez : ./build.sh windows"
    warn "Build Windows ignoré."
    return
  fi

  if ! rustup target list --installed 2>/dev/null | grep -q "$TARGET_WINDOWS"; then
    log "Ajout de la cible ${TARGET_WINDOWS} via rustup…"
    if ! rustup target add "$TARGET_WINDOWS"; then
      warn "Échec de 'rustup target add ${TARGET_WINDOWS}' (pas de connexion vers static.rust-lang.org ?)."
      warn "Build Windows ignoré."
      return
    fi
  fi

  if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    warn "mingw-w64 introuvable. Sur Debian/Ubuntu: sudo apt install mingw-w64"
    warn "Build Windows ignoré."
    return
  fi

  if ! CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
      cargo build --release --bins --target "$TARGET_WINDOWS" 2>"$DIST_DIR/windows-build.log"; then
    warn "Cross-compilation Windows échouée (voir dist/windows-build.log)."
    warn "Alternative fiable : builder nativement sur Windows (cargo build --release) ou via GitHub Actions (windows-latest, deja fourni dans .github/workflows/release.yml)."
    return
  fi

  local bin="target/${TARGET_WINDOWS}/release/${APP_NAME}.exe"
  local bin_server="target/${TARGET_WINDOWS}/release/${APP_NAME}-server.exe"
  local portable_dir="$DIST_DIR/${APP_NAME}-${VERSION}-windows-x86_64"
  rm -rf "$portable_dir"
  mkdir -p "$portable_dir"
  cp "$bin" "$portable_dir/"
  [ -f "$bin_server" ] && cp "$bin_server" "$portable_dir/"
  cp -r web "$portable_dir/"
  cp README.md LICENSE CHANGELOG.md "$portable_dir/" 2>/dev/null || true
  (cd "$DIST_DIR" && zip -rq "${APP_NAME}-${VERSION}-windows-x86_64.zip" "$(basename "$portable_dir")")
  ok "archive portable Windows: dist/${APP_NAME}-${VERSION}-windows-x86_64.zip"
  ok "binaire Windows: $bin"
}

# ------------------------------------------------------------------------------
build_installer() {
  log "Génération des installateurs…"

  # ---- Linux: script d'install universel (fallback si pas de .deb / autre distro) ----
  cat > "$DIST_DIR/install-linux.sh" << EOF
#!/usr/bin/env bash
set -euo pipefail
echo "Installation de MCManager ${VERSION}…"
DEST="\${1:-/usr/local}"
sudo mkdir -p "\$DEST/bin" "\$DEST/share/mcmanager"
sudo cp "\$(dirname "\$0")/${APP_NAME}-${VERSION}-linux-x86_64/mcmanager" "\$DEST/bin/mcmanager"
if [ -f "\$(dirname "\$0")/${APP_NAME}-${VERSION}-linux-x86_64/mcmanager-server" ]; then
  sudo cp "\$(dirname "\$0")/${APP_NAME}-${VERSION}-linux-x86_64/mcmanager-server" "\$DEST/bin/mcmanager-server"
  sudo chmod +x "\$DEST/bin/mcmanager-server"
fi
sudo cp -r "\$(dirname "\$0")/${APP_NAME}-${VERSION}-linux-x86_64/web" "\$DEST/share/mcmanager/web"
sudo chmod +x "\$DEST/bin/mcmanager"
echo "Installé. Lancez avec: mcmanager"
echo "Interface web: http://127.0.0.1:7777"
EOF
  chmod +x "$DIST_DIR/install-linux.sh"
  ok "installateur Linux: dist/install-linux.sh (alternative au .deb)"

  # ---- Windows: script Inno Setup (.iss). Compilez-le avec ISCC.exe (Inno Setup)
  # sur Windows ou via `wine ISCC.exe mcmanager.iss` pour obtenir un vrai .exe/.msi-like installer.
  cat > "$DIST_DIR/mcmanager.iss" << EOF
; Script Inno Setup pour MCManager ${VERSION}
; Compilation : ISCC.exe mcmanager.iss  (Inno Setup 6, https://jrsoftware.org/isinfo.php)
; Produit un installeur Windows classique (Suivant > Suivant > Installer),
; equivalent en usage a un .msi.
[Setup]
AppId={{9C2C6E2B-MCMANAGER-YOLEZZ-0001}}
AppName=MCManager
AppVersion=${VERSION}
AppPublisher=yolezz
DefaultDirName={autopf}\\MCManager
DefaultGroupName=MCManager
OutputDir=.
OutputBaseFilename=mcmanager-${VERSION}-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64
LicenseFile=LICENSE

[Files]
Source: "${APP_NAME}-${VERSION}-windows-x86_64\\mcmanager.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "${APP_NAME}-${VERSION}-windows-x86_64\\mcmanager-server.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "${APP_NAME}-${VERSION}-windows-x86_64\\web\\*"; DestDir: "{app}\\web"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\\MCManager"; Filename: "{app}\\mcmanager.exe"
Name: "{autodesktop}\\MCManager"; Filename: "{app}\\mcmanager.exe"

[Run]
Filename: "{app}\\mcmanager.exe"; Description: "Lancer MCManager"; Flags: postinstall nowait skipifsilent
EOF
  ok "script d'installateur Windows: dist/mcmanager.iss"
  if command -v iscc >/dev/null 2>&1; then
    (cd "$DIST_DIR" && iscc mcmanager.iss) && ok "installeur .exe généré" || warn "échec de compilation Inno Setup"
  else
    warn "Inno Setup (ISCC) non disponible ici : le .exe d'installation n'a PAS été généré dans cet environnement."
    warn "Sur une machine Windows (ou via GitHub Actions windows-latest), lancez: ISCC.exe dist/mcmanager.iss"
  fi
}

# ------------------------------------------------------------------------------
case "$TASK" in
  linux)     build_linux ;;
  windows)   build_windows ;;
  installer) build_installer ;;
  all)
    build_linux
    build_windows
    build_installer
    ;;
  *)
    err "tâche inconnue: $TASK (utilisez: linux|windows|installer|all)"
    exit 1
    ;;
esac

echo
log "Terminé. Artefacts disponibles dans: $DIST_DIR"
ls -lh "$DIST_DIR" 2>/dev/null || true
