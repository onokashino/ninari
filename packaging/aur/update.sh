#!/usr/bin/env bash
# Points PKGBUILD and .SRCINFO at the latest release: its version and checksum from latest.json.
set -euo pipefail
cd "$(dirname "$0")/ninari-bin"
meta="$(curl -fsSL https://github.com/onokashino/ninari/releases/latest/download/latest.json)"
version="$(printf '%s\n' "$meta" | sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p')"
sum="$(printf '%s\n' "$meta" | grep -A2 '"linux-x86_64"' | sed -n 's/.*"sha256": *"\([^"]*\)".*/\1/p')"
sed -i "s/^pkgver=.*/pkgver=$version/; s/^pkgrel=.*/pkgrel=1/; s/^sha256sums=.*/sha256sums=('$sum')/" PKGBUILD
makepkg --printsrcinfo > .SRCINFO
echo "ninari-bin $version"
