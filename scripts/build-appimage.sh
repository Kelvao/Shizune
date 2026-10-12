#!/usr/bin/env bash
set -euo pipefail

# AppDir preparation for Shizune
APPDIR="AppDir"
rm -rf "${APPDIR}"
mkdir -p "${APPDIR}/usr/bin"
mkdir -p "${APPDIR}/usr/share/icons/hicolor/scalable/apps"

cp target/release/shizune "${APPDIR}/usr/bin/shizune"
cp assets/shizune.desktop "${APPDIR}/shizune.desktop"
cp assets/shizune.svg "${APPDIR}/shizune.svg"
cp assets/shizune.svg "${APPDIR}/usr/share/icons/hicolor/scalable/apps/shizune.svg"

cat << 'EOF' > "${APPDIR}/AppRun"
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "${0}")")"
export PATH="${HERE}/usr/bin:${PATH}"
exec "${HERE}/usr/bin/shizune" "$@"
EOF
chmod +x "${APPDIR}/AppRun"

echo "AppDir created successfully at ${APPDIR}"

