#!/usr/bin/env bash
# Installs ninari for this user from an unpacked release: the program into
# ~/.local/share/ninari, the `ninari` command into ~/.local/bin, an entry in the app menu
# and its icons. Run it again to update; `install.sh --remove` takes it all away (the library and
# settings in ~/.local/share/com.onokashino.ninari stay).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
app="$data/ninari"
bin="$HOME/.local/bin"
entry="$data/applications/ninari.desktop"
sizes=(32 128 256 512)

if [ "${1:-}" = "--remove" ]; then
  rm -rf "$app"
  rm -f "$bin/ninari" "$entry"
  for s in "${sizes[@]}"; do
    rm -f "$data/icons/hicolor/${s}x${s}/apps/ninari.png"
  done
  echo "ninari removed"
  exit 0
fi

# A copy beside the old one, then a swap, so a failed copy leaves the old version working.
rm -rf "$app.new"
mkdir -p "$app.new" "$bin" "$data/applications"
# Everything the app needs is in the release, its libraries too: nothing from the system's mpv.
cp -r "$here/ninari" "$here/mpv" "$here/lib" "$here/mpv-config" "$app.new/"
rm -rf "$app"
mv "$app.new" "$app"
ln -sf "$app/ninari" "$bin/ninari"

for s in "${sizes[@]}"; do
  mkdir -p "$data/icons/hicolor/${s}x${s}/apps"
  cp "$here/icons/$s.png" "$data/icons/hicolor/${s}x${s}/apps/ninari.png"
done

cat >"$entry" <<EOF
[Desktop Entry]
Type=Application
Name=ninari
Comment=Аниме: каталог, библиотека и плеер
Exec="$app/ninari"
Icon=ninari
Terminal=false
Categories=AudioVideo;Video;Player;
StartupWMClass=ninari
EOF

command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$data/icons/hicolor" || true
echo "ninari installed: the app menu, or \`ninari\` in a terminal"
