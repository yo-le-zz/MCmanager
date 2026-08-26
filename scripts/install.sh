#!/usr/bin/env bash
# MCManager - script d'installation Linux (x86_64).
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
  rm -f "$BIN_DIR/mcmanager" "$BIN_DIR/mcmanager-headless"
  rm -f "$DESKTOP_FILE" "$ICON_FILE"
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$HOME/.local/share/applications" >/dev/null 2>&1 || true
  log "MCManager désinstallé."
  exit 0
fi

ARCH="$(uname -m)"
if [ "$ARCH" != "x86_64" ]; then
  err "Architecture non supportée pour le moment : $ARCH (seul x86_64 est fourni pour l'instant)."
  exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
  err "curl est requis pour cette installation."
  exit 1
fi

# ---- .deb en priorité si le système le supporte (intégration la plus propre : ----
# service systemd, désinstallation via le gestionnaire de paquets/logithèque) ----
if command -v dpkg >/dev/null 2>&1; then
  log "Système basé sur dpkg détecté - installation via le paquet .deb..."
  DEB_URL=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep -o 'https://[^"]*\.deb' | head -n1 || true)
  if [ -n "$DEB_URL" ]; then
    TMP=$(mktemp -d)
    trap 'rm -rf "$TMP"' EXIT
    log "Téléchargement de $(basename "$DEB_URL")..."
    curl -fsSL "$DEB_URL" -o "$TMP/mcmanager.deb"
    sudo dpkg -i "$TMP/mcmanager.deb" || sudo apt-get install -f -y
    log "Installé. Lancez avec la commande : mcmanager (ou cherchez \"MCManager\" dans votre menu applications)."
    log "Désinstaller : sudo apt remove mcmanager"
    exit 0
  fi
  log "Aucun .deb trouvé dans la dernière release, repli sur l'archive portable..."
fi

# ---- Repli portable : archive .tar.gz dans ~/.local ----
log "Installation portable dans $INSTALL_DIR..."
TAR_URL=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep -o 'https://[^"]*linux-x86_64\.tar\.gz' | head -n1 || true)
if [ -z "$TAR_URL" ]; then
  err "Impossible de trouver l'archive Linux dans la dernière release GitHub ($REPO)."
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
if [ -f "$INSTALL_DIR/mcmanager-headless" ]; then
  chmod +x "$INSTALL_DIR/mcmanager-headless"
  ln -sf "$INSTALL_DIR/mcmanager-headless" "$BIN_DIR/mcmanager-headless"
fi

# ---- Entrée dans le menu applications (touche Super sous GNOME/KDE etc) ----
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

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) log "Note : ajoutez $BIN_DIR à votre PATH (ex: echo 'export PATH=\"\$PATH:$BIN_DIR\"' >> ~/.bashrc) si la commande 'mcmanager' n'est pas trouvée." ;;
esac

log ""
log "MCManager installé."
log "Lancez avec la commande : mcmanager (ou cherchez \"MCManager\" dans votre menu applications)"
log "Interface web par défaut : http://127.0.0.1:7777"
log "Désinstaller : curl -fsSL https://mcmanager.pages.dev/install.sh | bash -s -- --uninstall"
