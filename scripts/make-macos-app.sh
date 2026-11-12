#!/usr/bin/env bash
# MCManager - assemble MCManager.app, l'archive portable et (sur macOS) l'image disque .dmg.
#
# Usage : scripts/make-macos-app.sh <version> <arch> <dossier-des-binaires> <dossier-de-sortie>
#   <arch> : arm64 (Apple Silicon) ou x86_64 (Intel) - sert seulement à nommer les fichiers.
#
# Produit dans <dossier-de-sortie> :
#   MCManager.app                              (signature « ad hoc », voir docs/MACOS.md)
#   mcmanager-<version>-macos-<arch>.tar.gz    (binaires + web, sans .app)
#   mcmanager-<version>-macos-<arch>.dmg       (uniquement si hdiutil existe, donc sur macOS)
#
# Le script n'a besoin que d'outils standards ; les étapes propres à macOS (iconutil, sips, codesign,
# hdiutil) sont ignorées quand elles sont absentes, ce qui permet de tester l'assemblage sous Linux.
set -euo pipefail

VERSION="${1:?version}"; ARCH="${2:?arch}"; BIN_DIR="${3:?dossier des binaires}"; OUT_DIR="${4:?dossier de sortie}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$OUT_DIR/MCManager.app"
MACOS_DIR="$APP/Contents/MacOS"; RES_DIR="$APP/Contents/Resources"

[ -f "$BIN_DIR/mcmanager" ] || { echo "binaire introuvable : $BIN_DIR/mcmanager" >&2; exit 1; }
rm -rf "$APP"; mkdir -p "$MACOS_DIR" "$RES_DIR"

cp "$BIN_DIR/mcmanager" "$MACOS_DIR/mcmanager"
[ -f "$BIN_DIR/mcmanager-server" ] && cp "$BIN_DIR/mcmanager-server" "$MACOS_DIR/mcmanager-server"
cp -R "$ROOT_DIR/web" "$RES_DIR/web"
chmod +x "$MACOS_DIR"/mcmanager*

# --- Lanceur : une application lancée depuis le Finder n'a pas le PATH d'un terminal (Homebrew absent, donc
#     pas de Java). On le complète, puis : si MCManager tourne déjà, on rouvre simplement le navigateur.
cat > "$MACOS_DIR/MCManager" <<'LAUNCHER'
#!/bin/bash
DIR="$(cd "$(dirname "$0")" && pwd)"
export PATH="/opt/homebrew/bin:/opt/homebrew/opt/openjdk/bin:/usr/local/bin:/usr/local/opt/openjdk/bin:$PATH"
export MCMANAGER_WEB_DIR="$DIR/../Resources/web"
PORT="${MCMANAGER_PORT:-7777}"
if curl -fsS --max-time 2 "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1; then
  open "http://127.0.0.1:$PORT"
  exit 0
fi
osascript -e "display notification \"Interface : http://127.0.0.1:$PORT (quitter : Moniteur d'activité > mcmanager)\" with title \"MCManager démarre\"" >/dev/null 2>&1 || true
exec "$DIR/mcmanager"
LAUNCHER
chmod +x "$MACOS_DIR/MCManager"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>MCManager</string>
  <key>CFBundleDisplayName</key><string>MCManager</string>
  <key>CFBundleIdentifier</key><string>io.github.yo-le-zz.mcmanager</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleExecutable</key><string>MCManager</string>
  <key>CFBundleIconFile</key><string>MCManager</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSUIElement</key><true/>
</dict>
</plist>
PLIST

# --- Icône .icns (iconutil + sips : macOS seulement) ---
if command -v iconutil >/dev/null 2>&1 && command -v sips >/dev/null 2>&1; then
  ICONSET="$(mktemp -d)/MCManager.iconset"; mkdir -p "$ICONSET"
  A="$ROOT_DIR/assets"
  cp "$A/icon-16.png"  "$ICONSET/icon_16x16.png"
  cp "$A/icon-32.png"  "$ICONSET/icon_16x16@2x.png"; cp "$A/icon-32.png" "$ICONSET/icon_32x32.png"
  cp "$A/icon-64.png"  "$ICONSET/icon_32x32@2x.png"
  sips -z 128 128 "$A/icon-256.png" --out "$ICONSET/icon_128x128.png" >/dev/null
  cp "$A/icon-256.png" "$ICONSET/icon_128x128@2x.png"; cp "$A/icon-256.png" "$ICONSET/icon_256x256.png"
  iconutil -c icns "$ICONSET" -o "$RES_DIR/MCManager.icns" || echo "avertissement : icône non générée" >&2
else
  echo "iconutil/sips absents : application sans icône .icns (normal hors macOS)" >&2
fi

# --- Signature « ad hoc » : obligatoire pour exécuter du code arm64 ; pas une notarisation (voir docs/MACOS.md) ---
if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP"
fi

# --- Archive portable ---
NAME="mcmanager-${VERSION}-macos-${ARCH}"
STAGE="$(mktemp -d)/$NAME"; mkdir -p "$STAGE"
cp "$BIN_DIR/mcmanager" "$STAGE/"; [ -f "$BIN_DIR/mcmanager-server" ] && cp "$BIN_DIR/mcmanager-server" "$STAGE/"
cp -R "$ROOT_DIR/web" "$STAGE/web"
cp "$ROOT_DIR/README.md" "$ROOT_DIR/LICENSE" "$ROOT_DIR/CHANGELOG.md" "$ROOT_DIR/docs/MACOS.md" "$STAGE/" 2>/dev/null || true
tar -C "$(dirname "$STAGE")" -czf "$OUT_DIR/$NAME.tar.gz" "$NAME"

# --- Image disque ---
if command -v hdiutil >/dev/null 2>&1; then
  DMG_STAGE="$(mktemp -d)"
  cp -R "$APP" "$DMG_STAGE/"; ln -s /Applications "$DMG_STAGE/Applications"
  cp "$ROOT_DIR/docs/MACOS.md" "$DMG_STAGE/LISEZMOI-macOS.md" 2>/dev/null || true
  hdiutil create -volname "MCManager ${VERSION}" -srcfolder "$DMG_STAGE" -ov -format UDZO "$OUT_DIR/$NAME.dmg" >/dev/null
fi
echo "OK : $OUT_DIR"; ls -1 "$OUT_DIR"
