#!/bin/bash
set -euo pipefail

VERSION="${1:-1.0.0}"
ARCH="${2:-x86_64}"
TARGET_DIR="dist"
RPMBUILD_DIR="/tmp/shizune-rpmbuild"

echo "==> Building RPM package for ${ARCH}..."
rm -rf "${RPMBUILD_DIR}"
mkdir -p "${RPMBUILD_DIR}"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

# Copy files to SOURCES
cp target/release/shizune "${RPMBUILD_DIR}/SOURCES/"
mkdir -p "${RPMBUILD_DIR}/SOURCES/assets"
cp assets/shizune.desktop "${RPMBUILD_DIR}/SOURCES/assets/"
cp assets/shizune.svg "${RPMBUILD_DIR}/SOURCES/assets/"
cp packaging/shizune.spec "${RPMBUILD_DIR}/SPECS/"

rpmbuild --define "_topdir ${RPMBUILD_DIR}" \
         --define "_sourcedir ${RPMBUILD_DIR}/SOURCES" \
         --define "pkg_version ${VERSION}" \
         --target "${ARCH}" \
         -bb "${RPMBUILD_DIR}/SPECS/shizune.spec"

mkdir -p "${TARGET_DIR}"
cp "${RPMBUILD_DIR}/RPMS/${ARCH}"/*.rpm "${TARGET_DIR}/"
rm -rf "${RPMBUILD_DIR}"
echo "==> RPM package generated in ${TARGET_DIR}/"

