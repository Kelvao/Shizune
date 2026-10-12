#!/bin/bash
set -euo pipefail

VERSION="${1:-1.0.0}"
ARCH="${2:-x86_64}"
TARGET_DIR="dist"
APPDIR="AppDir"

echo "==> Building AppImage for ${ARCH} (version: ${VERSION})..."
rm -rf "${APPDIR}"
mkdir -p "${APPDIR}/usr/bin"
mkdir -p "${APPDIR}/usr/share/applications"
mkdir -p "${APPDIR}/usr/share/icons/hicolor/scalable/apps"

cp target/release/shizune "${APPDIR}/usr/bin/shizune"
cp assets/shizune.desktop "${APPDIR}/shizune.desktop"
cp assets/shizune.desktop "${APPDIR}/usr/share/applications/shizune.desktop"
cp assets/shizune.svg "${APPDIR}/shizune.svg"
cp assets/shizune.svg "${APPDIR}/usr/share/icons/hicolor/scalable/apps/shizune.svg"

cat << 'EOF' > "${APPDIR}/AppRun"
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "${0}")")"
export PATH="${HERE}/usr/bin:${PATH}"
exec "${HERE}/usr/bin/shizune" "$@"
EOF
chmod +x "${APPDIR}/AppRun"

mkdir -p "${TARGET_DIR}"
if [ ! -d "appimagetool.AppDir" ]; then
    if [ ! -f "appimagetool-x86_64.AppImage" ]; then
        curl -sLO https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
        chmod +x appimagetool-x86_64.AppImage
    fi
    ./appimagetool-x86_64.AppImage --appimage-extract
    mv squashfs-root appimagetool.AppDir
fi

OUTPUT_FILE="${TARGET_DIR}/Shizune-${VERSION}-${ARCH}.AppImage"
ARCH="${ARCH}" ./appimagetool.AppDir/AppRun "${APPDIR}" "${OUTPUT_FILE}"
rm -rf "${APPDIR}"
echo "==> AppImage created: ${OUTPUT_FILE}"

