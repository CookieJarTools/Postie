#!/usr/bin/env bash
# Packs target/release/postie into dist/Postie-<version>.AppImage.
#
#   packaging/linux/build-appimage.sh 0.1.0
#
# Set APPIMAGETOOL to the appimagetool to use; it defaults to the one on PATH.
set -euo pipefail

version=${1:?Pass the version, such as 0.1.0}
tool=${APPIMAGETOOL:-appimagetool}
root=$(cd "$(dirname "$0")/../.." && pwd)
appdir=$(mktemp -d)/Postie.AppDir

mkdir -p "$appdir/usr/bin" "$appdir/usr/share/icons/hicolor/256x256/apps" "$appdir/usr/share/icons/hicolor/scalable/apps" "$root/dist"
install -m 755 "$root/target/release/postie" "$appdir/usr/bin/postie"
install -m 644 "$root/assets/icon.png" "$appdir/postie.png"
install -m 644 "$root/assets/icon.png" "$appdir/usr/share/icons/hicolor/256x256/apps/postie.png"
install -m 644 "$root/assets/icon.svg" "$appdir/usr/share/icons/hicolor/scalable/apps/postie.svg"
install -m 644 "$root/packaging/linux/postie.desktop" "$appdir/postie.desktop"
ln -s usr/bin/postie "$appdir/AppRun"

ARCH=x86_64 "$tool" --no-appstream "$appdir" "$root/dist/Postie-$version.AppImage"
echo "Built dist/Postie-$version.AppImage"
