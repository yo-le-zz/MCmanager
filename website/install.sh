#!/usr/bin/env bash
# MCManager - script d'installation Linux (x86_64, ARM64) et macOS (Intel, Apple Silicon).
# Usage :
#   curl -fsSL https://mcmanager.pages.dev/install.sh | bash
#   curl -fsSL https://mcmanager.pages.dev/install.sh | bash -s -- --uninstall
set -euo pipefail

REPO="yo-le-zz/MCmanager"
INSTALL_DIR="${MCMANAGER_INSTALL_DIR:-$HOME/.local/share/mcmanager}"
BIN_DIR="${MCMANAGER_BIN_DIR:-$HOME/.local/bin}"
DESKTOP_FILE="$HOME/.local/share/applications/mcmanager.desktop"
ICON_FILE="$HOME/.local/share/icons/hicolor/256x256/apps/mcmanager.png"

log()  { printf "\033[1;34m[mcmanager]\033[0m %s\n" "$*"; }
err()  { printf "\033[1;31m[mcmanager]\033[0m %s\n" "$*" >&2; }

# ---- désinstallation ----
if [ "${1:-}" = "--uninstall" ]; then
  log "Désinstallation de MCManager (installation portable ~/.local uniquement - si vous avez installé le .deb, utilisez 'sudo apt remove mcmanager')..."
  rm -rf "$INSTALL_DIR"
  rm -f "$BIN_DIR/mcmanager" "$BIN_DIR/mcmanager-server"
  rm -f "$DESKTOP_FILE" "$ICON_FILE"
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$HOME/.local/share/applications" >/dev/null 2>&1 || true
  log "MCManager désinstallé."
  exit 0
fi

OS="$(uname -s)"
case "$(uname -m)" in
  x86_64|amd64)  ARCH="x86_64"; DEB_ARCH="amd64"; RPM_ARCH="x86_64"; MAC_ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64"; DEB_ARCH="arm64"; RPM_ARCH="aarch64"; MAC_ARCH="x86_64" ;; # Apple Silicon : build Intel via Rosetta 2
  *) err "Architecture non supportée : $(uname -m) (fournies : x86_64 et ARM64 / Apple Silicon)."; exit 1 ;;
esac
case "$OS" in
  Linux|Darwin) ;;
  *) err "Système non supporté : $OS (Linux et macOS ; Windows : voir install.ps1)."; exit 1 ;;
esac
if ! command -v curl >/dev/null 2>&1; then
  err "curl est requis pour cette installation."
  exit 1
fi

RELEASE_JSON="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" || true)"
# Première URL de la release qui se termine par le motif donné (-> vide si absente).
asset_url() { printf '%s' "$RELEASE_JSON" | grep -o 'https://[^"]*' | grep -E "$1" | head -n1 || true; }

if [ "$OS" = "Linux" ]; then
  # ---- Paquet natif de la bonne architecture : service systemd, désinstallation par le gestionnaire de paquets ----
  if command -v dpkg >/dev/null 2>&1; then
    DEB_URL="$(asset_url "_${DEB_ARCH}\\.deb$|-${DEB_ARCH}\\.deb$")"
    if [ -n "$DEB_URL" ]; then
      log "Système basé sur dpkg détecté ($DEB_ARCH) - installation via le paquet .deb..."
      TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
      curl -fsSL "$DEB_URL" -o "$TMP/mcmanager.deb"
      sudo dpkg -i "$TMP/mcmanager.deb" || sudo apt-get install -f -y
      log "Installé. Lancez avec la commande : mcmanager (ou cherchez \"MCManager\" dans votre menu applications)."
      log "Désinstaller : sudo apt remove mcmanager"
      exit 0
    fi
    log "Aucun .deb $DEB_ARCH dans la dernière release, repli sur l'archive portable..."
  elif command -v rpm >/dev/null 2>&1; then
    RPM_URL="$(asset_url "\\.${RPM_ARCH}\\.rpm$")"
    if [ -n "$RPM_URL" ]; then
      log "Système basé sur rpm détecté ($RPM_ARCH) - installation via le paquet .rpm..."
      TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
      curl -fsSL "$RPM_URL" -o "$TMP/mcmanager.rpm"
      if command -v dnf >/dev/null 2>&1; then sudo dnf install -y "$TMP/mcmanager.rpm"
      elif command -v zypper >/dev/null 2>&1; then sudo zypper --non-interactive --no-gpg-checks install "$TMP/mcmanager.rpm"
      elif command -v yum >/dev/null 2>&1; then sudo yum install -y "$TMP/mcmanager.rpm"
      else sudo rpm -i "$TMP/mcmanager.rpm"; fi
      log "Installé. Lancez avec la commande : mcmanager"
      log "Désinstaller : sudo dnf remove mcmanager  (ou zypper / yum / rpm -e)"
      exit 0
    fi
    log "Aucun .rpm $RPM_ARCH dans la dernière release, repli sur l'archive portable..."
  fi
  PORTABLE_PATTERN="linux-${ARCH}\\.tar\\.gz$"
else
  if [ "$(uname -m)" = "arm64" ] && ! /usr/bin/pgrep -q oahd 2>/dev/null; then
    log "Mac Apple Silicon : MCManager est fourni pour Intel et tourne via Rosetta 2. Installez-la si besoin : softwareupdate --install-rosetta --agree-to-license"
  fi
  PORTABLE_PATTERN="macos-${MAC_ARCH}\\.tar\\.gz$"
fi

# ---- Repli portable : archive .tar.gz dans ~/.local ----
log "Installation portable dans $INSTALL_DIR..."
TAR_URL="$(asset_url "$PORTABLE_PATTERN")"
if [ -z "$TAR_URL" ]; then
  err "Impossible de trouver l'archive $OS ($(uname -m)) dans la dernière release GitHub ($REPO)."
  exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
log "Téléchargement de $(basename "$TAR_URL")..."
curl -fsSL "$TAR_URL" -o "$TMP/mcmanager.tar.gz"
tar xzf "$TMP/mcmanager.tar.gz" -C "$TMP"
SRC_DIR=$(find "$TMP" -maxdepth 1 -type d -name 'mcmanager-*' | head -n1)
if [ -z "$SRC_DIR" ]; then
  err "Archive inattendue (contenu introuvable)."
  exit 1
fi

mkdir -p "$INSTALL_DIR" "$BIN_DIR"
cp -r "$SRC_DIR"/. "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/mcmanager"
ln -sf "$INSTALL_DIR/mcmanager" "$BIN_DIR/mcmanager"
if [ -f "$INSTALL_DIR/mcmanager-server" ]; then
  chmod +x "$INSTALL_DIR/mcmanager-server"
  ln -sf "$INSTALL_DIR/mcmanager-server" "$BIN_DIR/mcmanager-server"
fi

# ---- Entrée dans le menu applications (Linux : touche Super sous GNOME/KDE etc) ----
if [ "$OS" = "Linux" ]; then
mkdir -p "$(dirname "$DESKTOP_FILE")" "$(dirname "$ICON_FILE")"
if [ -f "$INSTALL_DIR/web/assets/icon-256.png" ]; then
  cp "$INSTALL_DIR/web/assets/icon-256.png" "$ICON_FILE"
fi
cat > "$DESKTOP_FILE" << EOF
[Desktop Entry]
Type=Application
Name=MCManager
Comment=Gestionnaire de serveurs Minecraft
Exec=$BIN_DIR/mcmanager
Icon=mcmanager
Terminal=false
Categories=Game;Utility;Network;
EOF
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(dirname "$DESKTOP_FILE")" >/dev/null 2>&1 || true
fi

# macOS : ni Gatekeeper ni quarantaine pour un binaire téléchargé par curl, mais on lève l'attribut au cas où.
[ "$OS" = "Darwin" ] && xattr -dr com.apple.quarantine "$INSTALL_DIR" 2>/dev/null || true

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) log "Note : ajoutez $BIN_DIR à votre PATH (ex: echo 'export PATH=\"\$PATH:$BIN_DIR\"' >> ~/.bashrc) si la commande 'mcmanager' n'est pas trouvée." ;;
esac

log ""
log "MCManager installé."
log "Lancez avec la commande : mcmanager (ou cherchez \"MCManager\" dans votre menu applications)"
log "Interface web par défaut : http://127.0.0.1:7777"
log "Désinstaller : curl -fsSL https://mcmanager.pages.dev/install.sh | bash -s -- --uninstall"
