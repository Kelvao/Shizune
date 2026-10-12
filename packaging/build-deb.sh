#!/bin/bash
set -euo pipefail

VERSION="${1:-1.0.0}"
ARCH="${2:-amd64}"
TARGET_DIR="dist"
PKG_NAME="shizune"
PKG_DIR="${TARGET_DIR}/${PKG_NAME}_${VERSION}_${ARCH}"

echo "==> Building Debian package (.deb) for ${ARCH}..."
rm -rf "${PKG_DIR}"
mkdir -p "${PKG_DIR}/DEBIAN"
mkdir -p "${PKG_DIR}/usr/bin"
mkdir -p "${PKG_DIR}/usr/share/applications"
mkdir -p "${PKG_DIR}/usr/share/icons/hicolor/scalable/apps"

# Copy binary
cp target/release/shizune "${PKG_DIR}/usr/bin/shizune"
chmod 755 "${PKG_DIR}/usr/bin/shizune"

# Copy desktop file and icon
cp assets/shizune.desktop "${PKG_DIR}/usr/share/applications/shizune.desktop"
cp assets/shizune.svg "${PKG_DIR}/usr/share/icons/hicolor/scalable/apps/shizune.svg"

# Generate control file
cat <<EOF > "${PKG_DIR}/DEBIAN/control"
Package: ${PKG_NAME}
Version: ${VERSION}
Section: sound
Priority: optional
Architecture: ${ARCH}
Maintainer: Kelvao <https://github.com/Kelvao/Shizune>
Description: High-performance AI noise suppression virtual microphone for Linux
 Real-time deep filter microphone noise reduction creating a PipeWire virtual source.
Recommends: pipewire (>= 0.3) | pulseaudio
EOF

dpkg-deb --build --root-owner-group "${PKG_DIR}" "${TARGET_DIR}/${PKG_NAME}_${VERSION}_${ARCH}.deb"
rm -rf "${PKG_DIR}"
echo "==> Debian package created: ${TARGET_DIR}/${PKG_NAME}_${VERSION}_${ARCH}.deb"

