#!/usr/bin/env bash
# A Linux release: builds the app and packs dist/ninari-<version>-linux-x86_64.tar.gz, which
# carries everything the app needs, so it does not depend on the system's mpv:
#
#   ninari     the program; it looks for its libraries in lib/ first
#   mpv          mpv for X11 sessions, with the same lib/
#   lib/         libmpv and the libraries it needs, ffmpeg and the codecs among them
#   mpv-config/  mpv's settings and the upscaling shaders
#   icons/, install.sh
#
# Left to the system, as AppImages do: glibc, the C++ runtime, the GPU driver's libraries (GL,
# EGL, Vulkan, VA-API, DRM), X11 and Wayland, the sound servers, and glib with gdk-pixbuf, which
# load the system's own plugins (VA-API drivers, image loaders) and must match them. Every
# desktop has those.
#
# The release workflow runs this in an Arch Linux container (needs mpv and patchelf): the mpv
# packed is the one the app is developed and tried with. An older one breaks it (mpv 0.37 took
# loadfile's arguments in another order).
set -Eeuo pipefail
# A failed step names itself among the run's annotations on GitHub.
trap 'echo "::error::scripts/package.sh line $LINENO: $BASH_COMMAND" >&2' ERR

root="$(cd "$(dirname "$0")/.." && pwd)"
target="${CARGO_TARGET_DIR:-$root/target}"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/crates/app/Cargo.toml" | head -1)"
name="ninari-$version-linux-x86_64"
out="$root/dist/$name"
system_mpv="$(command -v mpv)" || { echo "mpv is needed to pack it" >&2; exit 1; }
command -v patchelf >/dev/null || { echo "patchelf is needed (apt install patchelf)" >&2; exit 1; }

if [ ! -d "$root/crates/app/mpv-config/shaders" ]; then
  bash "$root/scripts/fetch-shaders.sh"
fi
# DT_RPATH, not RUNPATH: it also finds the libraries of the libraries, libmpv's own.
(cd "$root/crates/app" &&
  CARGO_TARGET_DIR="$target" \
  RUSTFLAGS="-C link-arg=-Wl,--disable-new-dtags,-rpath,\$ORIGIN/lib" \
  cargo build --release --bin ninari)

rm -rf "$out"
mkdir -p "$out/lib" "$out/icons"
cp "$target/release/ninari" "$out/"
cp "$system_mpv" "$out/mpv"
patchelf --force-rpath --set-rpath '$ORIGIN/lib' "$out/mpv"

# What stays the system's (see the top).
keep_system='^(ld-linux|libc\.so|libm\.so|libdl\.so|libpthread|librt\.so|libresolv|libutil|libmvec|libgcc_s|libstdc\+\+|libGL|libEGL|libOpenGL|libglapi|libgbm|libdrm|libvulkan|libva|libvdpau|libnvidia|libcuda|libOpenCL|libX|libxcb|libxkbcommon|libwayland|libdbus|libsystemd|libasound|libpulse|libpipewire|libjack|libfontconfig|libfreetype|libexpat|libz\.so|libglib|libgobject|libgio|libgmodule|libgdk_pixbuf)'
for elf in "$out/ninari" "$out/mpv"; do
  ldd "$elf" | while read -r first arrow path _; do
    # "libfoo.so.1 => /usr/lib/libfoo.so.1", or a path alone; the loader comes as a path on
    # either side ("/lib64/ld-linux-x86-64.so.2 => /usr/lib64/...").
    if [ "$arrow" = "=>" ]; then
      lib="$(basename "$first")"
    elif [[ "$first" == /* ]]; then
      lib="$(basename "$first")" path="$first"
    else
      continue
    fi
    if [ "$path" = "not" ]; then
      echo "$elf: $lib not found" >&2
      exit 1
    fi
    if [[ ! "$lib" =~ $keep_system ]] && [ ! -e "$out/lib/$lib" ]; then
      cp -L "$path" "$out/lib/$lib"
    fi
  done
done

cp -r "$root/crates/app/mpv-config" "$out/"
cp "$root/crates/app/icons/32x32.png" "$out/icons/32.png"
cp "$root/crates/app/icons/128x128.png" "$out/icons/128.png"
cp "$root/crates/app/icons/128x128@2x.png" "$out/icons/256.png"
cp "$root/crates/app/icons/icon.png" "$out/icons/512.png"
cp "$root/scripts/install.sh" "$out/"

# The program must find libmpv in its own lib/, not the system's.
ldd "$out/ninari" | grep -q "$out/lib/libmpv" || { echo "libmpv is not taken from lib/" >&2; exit 1; }
tar -C "$root/dist" -czf "$root/dist/$name.tar.gz" "$name"
echo "$root/dist/$name.tar.gz ($(du -sh "$out/lib" | cut -f1) of libraries)"
