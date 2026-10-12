#!/bin/bash
set -euo pipefail

VERSION="${1:-1.0.0}"
ARCH="${2:-x86_64}"
TARGET_DIR="dist"
APP_DIR="/tmp/Shizune.AppDir"

echo "==> Building AppImage for ${ARCH} (version: ${VERSION})..."
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/usr/bin"
mkdir -p "${APP_DIR}/usr/share/applications"
mkdir -p "${APP_DIR}/usr/share/icons/hicolor/scalable/apps"

# Copy binary
BIN_PATH="target/release/shizune"
if [ ! -f "${BIN_PATH}" ]; then
    BIN_PATH="shizune"
fi
cp "${BIN_PATH}" "${APP_DIR}/usr/bin/shizune"
chmod 755 "${APP_DIR}/usr/bin/shizune"

# Copy desktop file and icons
cp assets/shizune.desktop "${APP_DIR}/shizune.desktop"
cp assets/shizune.desktop "${APP_DIR}/usr/share/applications/shizune.desktop"
echo "X-AppImage-Version=${VERSION}" >> "${APP_DIR}/shizune.desktop"
echo "X-AppImage-Version=${VERSION}" >> "${APP_DIR}/usr/share/applications/shizune.desktop"

cp assets/shizune.svg "${APP_DIR}/shizune.svg"
cp assets/shizune.svg "${APP_DIR}/.DirIcon"
cp assets/shizune.svg "${APP_DIR}/usr/share/icons/hicolor/scalable/apps/shizune.svg"

# Create AppStream metainfo for software centers & Gear Lever
mkdir -p "${APP_DIR}/usr/share/metainfo"
cat <<EOF > "${APP_DIR}/usr/share/metainfo/io.github.kelvao.shizune.metainfo.xml"
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>io.github.kelvao.shizune</id>
  <metadata_license>MIT</metadata_license>
  <project_license>MIT</project_license>
  <name>Shizune</name>
  <summary>High-performance AI noise suppression virtual microphone for Linux</summary>
  <description>
    <p>
      Real-time deep filter microphone noise reduction creating an isolated
      virtual microphone source on Linux. Features a modern Slint GUI control panel.
    </p>
  </description>
  <launchable type="desktop-id">shizune.desktop</launchable>
  <url type="homepage">https://github.com/Kelvao/Shizune</url>
  <url type="bugtracker">https://github.com/Kelvao/Shizune/issues</url>
  <developer id="io.github.kelvao">
    <name>Kelvão</name>
  </developer>
  <provides>
    <binary>shizune</binary>
  </provides>
  <content_rating type="oars-1.1" />
</component>
EOF

# AppRun script
cat <<'EOF' > "${APP_DIR}/AppRun"
#!/bin/sh
HERE="$(dirname "$(readlink -f "${0}")")"
export PATH="${HERE}/usr/bin:${PATH}"
exec "${HERE}/usr/bin/shizune" "$@"
EOF
chmod +x "${APP_DIR}/AppRun"

mkdir -p "${TARGET_DIR}"
if ! command -v appimagetool >/dev/null 2>&1; then
    if [ ! -f /tmp/squashfs-root/AppRun ]; then
        echo "Downloading appimagetool..."
        curl -fsSL -o /tmp/appimagetool "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-${ARCH}.AppImage"
        chmod +x /tmp/appimagetool
        (cd /tmp && ./appimagetool --appimage-extract)
    fi
    TOOL="/tmp/squashfs-root/AppRun"
else
    TOOL="appimagetool"
fi

UPDATE_INFO="gh-releases-zsync|Kelvao|Shizune|latest|Shizune-*${ARCH}.AppImage.zsync"

OUTPUT_FILE="${TARGET_DIR}/Shizune-${VERSION}-${ARCH}.AppImage"
ARCH="${ARCH}" "${TOOL}" -u "${UPDATE_INFO}" "${APP_DIR}" "${OUTPUT_FILE}" || \
ARCH="${ARCH}" "${TOOL}" "${APP_DIR}" "${OUTPUT_FILE}"

rm -rf "${APP_DIR}"
echo "==> AppImage created: ${OUTPUT_FILE}"
