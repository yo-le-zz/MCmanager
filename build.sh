#!/usr/bin/env bash
# ==============================================================================
#  MCManager - build.sh
#  Compile et empaquette MCManager pour Linux (.deb)
#  et Windows (binaire + installeur Inno Setup), puis regroupe tous les
#  artefacts dans ./dist/.
#
#  Usage:
#     ./build.sh                # build tout ce qui est possible sur cette machine
#     ./build.sh linux          # uniquement le binaire + .deb Linux (x86_64)
#     ./build.sh linux-arm64    # Linux ARM64 : Raspberry Pi 4/5, VPS ARM (cross-compilation)
#     ./build.sh windows        # cross-build Windows x86_64
#     ./build.sh windows-arm64  # cross-build Windows ARM64 (zig)
#     ./build.sh all            # tout : Linux x86_64 + ARM64, Windows x86_64 + ARM64, installateurs
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
  # $1 : x86_64 (défaut) ou aarch64 (Raspberry Pi 4/5, serveurs ARM, VPS ARM)
  local arch_name="${1:-x86_64}"
  local deb_arch="amd64" cargo_args=(--release --bins) bin_dir="target/release"
  if [ "$arch_name" = "aarch64" ]; then
    deb_arch="arm64"
    bin_dir="target/aarch64-unknown-linux-gnu/release"
    cargo_args=(--release --bins --target aarch64-unknown-linux-gnu)
    if [ "$(uname -m)" != "aarch64" ] && [ "${SKIP_CARGO:-0}" != "1" ]; then
      setup_arm_cross || return 1
    fi
  fi
  if [ "${SKIP_CARGO:-0}" = "1" ]; then
    log "Compilation ignorée (SKIP_CARGO=1) : utilisation de $bin_dir"
  else
    log "Compilation du binaire Linux ${arch_name} (release)…"
    cargo build "${cargo_args[@]}"
  fi
  local bin="${bin_dir}/${APP_NAME}"
  local bin_server="${bin_dir}/${APP_NAME}-server"
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
  local portable_dir="$DIST_DIR/${APP_NAME}-${VERSION}-linux-${arch_name}"
  rm -rf "$portable_dir"
  mkdir -p "$portable_dir"
  cp "$bin" "$portable_dir/"
  [ -f "$bin_server" ] && cp "$bin_server" "$portable_dir/"
  cp -r web "$portable_dir/"
  cp README.md LICENSE CHANGELOG.md "$portable_dir/" 2>/dev/null || true
  (cd "$DIST_DIR" && tar czf "${APP_NAME}-${VERSION}-linux-${arch_name}.tar.gz" "$(basename "$portable_dir")")
  ok "archive portable: dist/${APP_NAME}-${VERSION}-linux-${arch_name}.tar.gz"

  build_deb "$bin" "$bin_server" "$deb_arch"
}

# Cross-compilation vers ARM64 depuis une machine x86_64 : cible rustup + un
# linker. Avec `zig` (pip install ziglang) le binaire reste compatible avec
# les glibc anciennes (2.31+ : Raspberry Pi OS, Ubuntu 20.04+) ; à défaut, le
# gcc croisé du système (apt install gcc-aarch64-linux-gnu).
setup_arm_cross() {
  if command -v rustup >/dev/null 2>&1; then
    rustup target add aarch64-unknown-linux-gnu >/dev/null 2>&1 || true
  fi
  if python3 -m ziglang version >/dev/null 2>&1; then
    local wrap; wrap="$(mktemp -d)"
    printf '#!/bin/bash\nargs=(); for a in "$@"; do case "$a" in --target=*|-target) ;; *) args+=("$a");; esac; done\nexec python3 -m ziglang cc -target aarch64-linux-gnu.2.31 "${args[@]}"\n' > "$wrap/cc"
    printf '#!/bin/bash\nexec python3 -m ziglang ar "$@"\n' > "$wrap/ar"
    chmod +x "$wrap/cc" "$wrap/ar"
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="$wrap/cc" CC_aarch64_unknown_linux_gnu="$wrap/cc" AR_aarch64_unknown_linux_gnu="$wrap/ar"
    ok "cross-compilation ARM64 via zig (glibc 2.31+)"
  elif command -v aarch64-linux-gnu-gcc >/dev/null 2>&1; then
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
    ok "cross-compilation ARM64 via aarch64-linux-gnu-gcc"
  else
    err "aucun linker ARM64 : installez 'pip install ziglang' ou 'apt install gcc-aarch64-linux-gnu'"; return 1
  fi
}

# ------------------------------------------------------------------------------
build_deb() {
  local bin="$1"
  local bin_server="${2:-}"
  local arch_override="${3:-}"
  if ! command -v dpkg-deb >/dev/null 2>&1; then
    warn "dpkg-deb introuvable, paquet .deb ignoré (installez dpkg-dev)."
    return
  fi
  log "Construction du paquet .deb…"
  local arch
  arch="${arch_override:-$(dpkg --print-architecture 2>/dev/null || echo amd64)}"
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
  # $1 : x86_64 (defaut) ou aarch64 (Windows sur ARM : Surface Pro X, PC Copilot+...)
  local arch_name="${1:-x86_64}"
  local target folder_arch
  if [ "$arch_name" = "aarch64" ]; then
    target="aarch64-pc-windows-gnullvm"; folder_arch="arm64"
  else
    target="$TARGET_WINDOWS"; folder_arch="x86_64"
  fi
  log "Tentative de cross-compilation Windows ${arch_name} (${target})…"

  # IMPORTANT : le rustc fourni par les gestionnaires de paquets Linux (apt,
  # dnf, pacman...) n'embarque QUE la std pour votre plateforme native. La std
  # Windows n'est distribuee que via rustup.
  if ! command -v rustup >/dev/null 2>&1; then
    warn "rustup est introuvable (la std Windows n'est distribuee que par rustup : https://rustup.rs)."
    warn "Build Windows ${arch_name} ignore. Alternative : GitHub Actions (runners Windows natifs, voir .github/workflows/release.yml)."
    return 0
  fi
  if ! rustup target list --installed 2>/dev/null | grep -q "$target"; then
    log "Ajout de la cible ${target} via rustup…"
    if ! rustup target add "$target"; then
      warn "Echec de 'rustup target add ${target}'. Build Windows ${arch_name} ignore."
      return 0
    fi
  fi

  if [ "$arch_name" = "aarch64" ]; then
    # Linker / compilateur C : zig (pip install ziglang) sait cibler Windows ARM64.
    if ! python3 -m ziglang version >/dev/null 2>&1; then
      warn "zig introuvable (pip install ziglang) : build Windows ARM64 ignore. Alternative : GitHub Actions (runner windows-11-arm)."
      return 0
    fi
    local wrap
    wrap="$(mktemp -d)"
    printf '#!/bin/bash\nargs=(); for a in "$@"; do case "$a" in --target=*|-target) ;; *) args+=("$a");; esac; done\nexec python3 -m ziglang cc -target aarch64-windows-gnu "${args[@]}"\n' > "$wrap/cc"
    printf '#!/bin/bash\nexec python3 -m ziglang ar "$@"\n' > "$wrap/ar"
    chmod +x "$wrap/cc" "$wrap/ar"
    export CARGO_TARGET_AARCH64_PC_WINDOWS_GNULLVM_LINKER="$wrap/cc" CC_aarch64_pc_windows_gnullvm="$wrap/cc" AR_aarch64_pc_windows_gnullvm="$wrap/ar"
  else
    if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
      warn "mingw-w64 introuvable. Sur Debian/Ubuntu: sudo apt install mingw-w64"
      warn "Build Windows x86_64 ignore."
      return 0
    fi
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
  fi

  if ! cargo build --release --bins --target "$target" 2>"$DIST_DIR/windows-${folder_arch}-build.log"; then
    warn "Cross-compilation Windows ${arch_name} echouee (voir dist/windows-${folder_arch}-build.log)."
    warn "Alternative fiable : GitHub Actions (runners Windows natifs, deja fournis dans .github/workflows/release.yml)."
    return 0
  fi

  local bin="target/${target}/release/${APP_NAME}.exe"
  local bin_server="target/${target}/release/${APP_NAME}-server.exe"
  local portable_dir="$DIST_DIR/${APP_NAME}-${VERSION}-windows-${folder_arch}"
  rm -rf "$portable_dir"
  mkdir -p "$portable_dir"
  cp "$bin" "$portable_dir/"
  [ -f "$bin_server" ] && cp "$bin_server" "$portable_dir/"
  cp -r web "$portable_dir/"
  cp README.md LICENSE CHANGELOG.md "$portable_dir/" 2>/dev/null || true
  (cd "$DIST_DIR" && zip -rq "${APP_NAME}-${VERSION}-windows-${folder_arch}.zip" "$(basename "$portable_dir")")
  ok "archive portable Windows: dist/${APP_NAME}-${VERSION}-windows-${folder_arch}.zip"
}

# ------------------------------------------------------------------------------
# Ecrit dist/mcmanager-<arch>.iss pour un dossier portable Windows existant.
emit_iss() {
  local folder_arch="$1" allowed install64
  allowed="x64"; install64="x64"
  if [ "$folder_arch" = "arm64" ]; then allowed="arm64"; install64="arm64"; fi
  local portable="${APP_NAME}-${VERSION}-windows-${folder_arch}"
  cat > "$DIST_DIR/mcmanager-${folder_arch}.iss" << ISSEOF
; Script Inno Setup pour MCManager ${VERSION} (${folder_arch})
; Compilation : ISCC.exe mcmanager-${folder_arch}.iss  (Inno Setup 6, https://jrsoftware.org/isinfo.php)
; Produit un installateur Windows classique (Suivant > Suivant > Installer).
[Setup]
AppId={{9C2C6E2B-4D43-4A52-9E11-4D434D414E01}
AppName=MCManager
AppVersion=${VERSION}
AppPublisher=yolezz
DefaultDirName={autopf}\\MCManager
DefaultGroupName=MCManager
OutputDir=.
OutputBaseFilename=mcmanager-${VERSION}-windows-${folder_arch}-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=${allowed}
ArchitecturesInstallIn64BitMode=${install64}
LicenseFile=${portable}\\LICENSE
PrivilegesRequiredOverridesAllowed=dialog

[Files]
Source: "${portable}\\mcmanager.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "${portable}\\mcmanager-server.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "${portable}\\web\\*"; DestDir: "{app}\\web"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\\MCManager"; Filename: "{app}\\mcmanager.exe"
Name: "{autodesktop}\\MCManager"; Filename: "{app}\\mcmanager.exe"

[Run]
Filename: "{app}\\mcmanager.exe"; Description: "Lancer MCManager"; Flags: postinstall nowait skipifsilent
ISSEOF
  ok "script d'installateur Windows: dist/mcmanager-${folder_arch}.iss"
}

# ------------------------------------------------------------------------------
build_installer() {
  log "Generation des installateurs…"

  # ---- Linux: script d'install universel (x86_64 et ARM64) ----
  cat > "$DIST_DIR/install-linux.sh" << LNXEOF
#!/usr/bin/env bash
set -euo pipefail
case "\$(uname -m)" in
  x86_64) ARCH=x86_64 ;;
  aarch64|arm64) ARCH=aarch64 ;;
  *) echo "Architecture non supportee : \$(uname -m)"; exit 1 ;;
esac
SRC="\$(dirname "\$0")/${APP_NAME}-${VERSION}-linux-\$ARCH"
[ -d "\$SRC" ] || { echo "Dossier introuvable : \$SRC (decompressez d'abord l'archive linux-\$ARCH)"; exit 1; }
echo "Installation de MCManager ${VERSION} (\$ARCH)…"
DEST="\${1:-/usr/local}"
sudo mkdir -p "\$DEST/bin" "\$DEST/share/mcmanager"
sudo cp "\$SRC/mcmanager" "\$DEST/bin/mcmanager"
sudo chmod +x "\$DEST/bin/mcmanager"
if [ -f "\$SRC/mcmanager-server" ]; then
  sudo cp "\$SRC/mcmanager-server" "\$DEST/bin/mcmanager-server"
  sudo chmod +x "\$DEST/bin/mcmanager-server"
fi
sudo rm -rf "\$DEST/share/mcmanager/web"
sudo cp -r "\$SRC/web" "\$DEST/share/mcmanager/web"
echo "Installe. Lancez avec: mcmanager   (mode serveur : mcmanager-server --daemon)"
echo "Interface web: http://127.0.0.1:7777"
LNXEOF
  chmod +x "$DIST_DIR/install-linux.sh"
  ok "installateur Linux: dist/install-linux.sh (alternative au .deb, x86_64 et ARM64)"

  # ---- Windows : un installateur Inno Setup par architecture dont le dossier portable existe ----
  local any=0 fa
  for fa in x86_64 arm64; do
    if [ -d "$DIST_DIR/${APP_NAME}-${VERSION}-windows-${fa}" ]; then
      emit_iss "$fa"; any=1
      if command -v iscc >/dev/null 2>&1 || command -v ISCC.exe >/dev/null 2>&1; then
        if (cd "$DIST_DIR" && { iscc "mcmanager-${fa}.iss" 2>/dev/null || ISCC.exe "mcmanager-${fa}.iss"; }); then
          ok "installateur .exe genere (${fa})"
        else
          warn "echec de compilation Inno Setup (${fa})"
        fi
      else
        warn "Inno Setup (ISCC) non disponible ici : l'installateur .exe (${fa}) sera produit par GitHub Actions, ou sur Windows : ISCC.exe dist/mcmanager-${fa}.iss"
      fi
    fi
  done
  [ "$any" = 1 ] || warn "aucun dossier portable Windows dans dist/ : lancez d'abord './build.sh windows' (ou 'windows-arm64')."
}

# ------------------------------------------------------------------------------
case "$TASK" in
  linux)     build_linux x86_64 ;;
  linux-arm64|arm64) build_linux aarch64 ;;
  windows)   build_windows x86_64 ;;
  windows-arm64) build_windows aarch64 ;;
  installer) build_installer ;;
  all)
    # Tout ce qui est possible sur cette machine : une etape qui manque d'un
    # outil (rustup, mingw, zig...) previent puis passe a la suivante.
    build_linux x86_64
    build_linux aarch64 || warn "Linux ARM64 ignore"
    build_windows x86_64
    build_windows aarch64
    build_installer
    ;;
  *)
    err "tâche inconnue: $TASK (utilisez: linux|linux-arm64|windows|windows-arm64|installer|all)"
    exit 1
    ;;
esac

echo
log "Terminé. Artefacts disponibles dans: $DIST_DIR"
ls -lh "$DIST_DIR" 2>/dev/null || true
