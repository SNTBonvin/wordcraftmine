#!/usr/bin/env bash
# Build and package WordCraft for Linux x86_64:
#
#   $DIST/wordcraft-<version>-linux-x86_64.deb   Debian, Ubuntu, Mint, Pop!_OS, ...
#
# This repository ships one build target only: Linux x86_64 as a .deb (amd64).
#
# Usage: packaging/linux/package.sh [--skip-build]
#
# Needs: cargo; nfpm for the deb (https://nfpm.goreleaser.com/install/). Build on an old
# distro (CI: Ubuntu 22.04, glibc 2.35) so the binary runs on newer ones.
# Optional: desktop-file-validate, appstreamcli.
set -euo pipefail
umask 022
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"
APP_ID=ai.storyteller.wordcraft

SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,9p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="$(uname -m)"
if [ "$ARCH" != "x86_64" ]; then
  echo "unsupported architecture $ARCH: this repository only builds Linux x86_64" >&2
  exit 2
fi
export WORDCRAFT_MAINTAINER="${WORDCRAFT_MAINTAINER:-WordCraft maintainers <wordcraft@storyteller.ai>}"
BASENAME="wordcraft-$VERSION-linux-$ARCH"

echo "==> WordCraft $VERSION for Linux $ARCH (.deb)"

if [ "$SKIP_BUILD" = 0 ]; then
  (cd "$ROOT" && cargo build --release --locked -p wordcraft -p wordcraft-cli)
fi
BIN="$CARGO_TARGET_DIR/release"
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
rm -rf "$WORK"

# ---- stage an FHS tree --------------------------------------------------------------------------
install -Dm755 "$BIN/wordcraft" "$STAGE/usr/bin/wordcraft"
install -Dm755 "$BIN/wordcraft-cli" "$STAGE/usr/bin/wordcraft-cli"
strip "$STAGE/usr/bin/wordcraft" "$STAGE/usr/bin/wordcraft-cli" 2>/dev/null || true
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$HERE/$APP_ID.mime.xml" "$STAGE/usr/share/mime/packages/$APP_ID.xml"
mkdir -p "$STAGE/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$WORDCRAFT_BUILD_DATE/g" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
mkdir -p "$STAGE/usr/share/icons"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/usr/share/icons/"
mkdir -p "$STAGE/usr/share/doc/wordcraft"
copy_docs "$STAGE/usr/share/doc/wordcraft"
install -Dm644 "$HERE/copyright" "$STAGE/usr/share/doc/wordcraft/copyright"
# Minimal Debian changelog (nfpm writes no upstream changelog of its own).
CHANGE_DATE="$(date -u -d "$WORDCRAFT_BUILD_DATE" +"%a, %d %b %Y 00:00:00 +0000")"
{
  echo "wordcraft ($VERSION-1) stable; urgency=medium"
  echo
  echo "  * WordCraft $VERSION for Linux x86_64."
  echo
  echo " -- WordCraft maintainers <wordcraft@storyteller.ai>  $CHANGE_DATE"
} | gzip -9n >"$STAGE/usr/share/doc/wordcraft/changelog.Debian.gz"
chmod 644 "$STAGE/usr/share/doc/wordcraft/changelog.Debian.gz"
mkdir -p "$STAGE/usr/share/man/man1"
for m in wordcraft wordcraft-cli; do
  gzip -9n -c "$HERE/$m.1" >"$STAGE/usr/share/man/man1/$m.1.gz"
  chmod 644 "$STAGE/usr/share/man/man1/$m.1.gz"
done

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
fi

# ---- .deb ---------------------------------------------------------------------------------------
command -v nfpm >/dev/null || { echo "error: nfpm not found (https://nfpm.goreleaser.com/install/)" >&2; exit 1; }
export VERSION
export NFPM_ARCH="amd64"
# nfpm expands env vars in fields like `version` and `arch`, but not in `contents[].src`.
sed "s|\${STAGE}|$STAGE|g" "$HERE/nfpm.yaml" >"$WORK/nfpm.yaml"
(cd "$ROOT" && nfpm package -f "$WORK/nfpm.yaml" -p deb -t "$DIST/$BASENAME.deb")

"$STAGE/usr/bin/wordcraft-cli" --version
echo "==> done"
ls -lh "$DIST"
