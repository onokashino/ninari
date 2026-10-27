#!/usr/bin/env bash
# Installs or updates ninari from its latest release, checked against the release's checksum:
#   curl -fsSL https://onokashino.github.io/ninari/install.sh | bash
# Arguments go to the release's own install.sh: `| bash -s -- --remove` takes ninari away.
set -euo pipefail

latest="https://github.com/onokashino/ninari/releases/latest/download/latest.json"
meta="$(curl -fsSL "$latest")"
# latest.json is jq's pretty output: the Linux entry's url and sha256 are the two lines after it.
entry="$(printf '%s\n' "$meta" | grep -A2 '"linux-x86_64"')"
url="$(printf '%s\n' "$entry" | sed -n 's/.*"url": *"\([^"]*\)".*/\1/p')"
sum="$(printf '%s\n' "$entry" | sed -n 's/.*"sha256": *"\([^"]*\)".*/\1/p')"
if [ -z "$url" ] || [ -z "$sum" ]; then
  echo "ninari: no Linux package in $latest" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
echo "ninari: downloading ${url##*/}"
curl -fL --progress-bar -o "$work/ninari.tar.gz" "$url"
echo "$sum  $work/ninari.tar.gz" | sha256sum -c --quiet -
tar -xzf "$work/ninari.tar.gz" -C "$work"
"$work"/ninari-*/install.sh "$@"
