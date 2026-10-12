#!/usr/bin/env bash
# WordCraft build and package helper.
# Usage: ./build.sh [--skip-build] [--formats "deb tar appimage rpm"]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$HERE/packaging/linux/package.sh" "$@"

