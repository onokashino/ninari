#!/usr/bin/env bash
# The player's upscaling shaders, Anime4K and ArtCNN, into crates/app/mpv-config/shaders
# (git-ignored). Run once after cloning; scripts/fetch-mpv.ps1 does the same on Windows.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dir="$root/crates/app/mpv-config/shaders"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$dir"

curl -fsSL -o "$tmp/anime4k.zip" \
  https://github.com/bloc97/Anime4K/releases/download/v4.0.1/Anime4K_v4.0.zip
unzip -oq "$tmp/anime4k.zip" -d "$dir"

# Pinned: the version these shaders were tried with.
artcnn=0837e5326e710f49e2ab0a8d995da0708047442e
for name in C4F16 C4F16_DS C4F16_DN C4F32 C4F32_DS C4F32_DN; do
  # Two fixes for mpv's OpenGL renderer, the one libmpv draws with (ArtCNN targets gpu-next):
  # it puts a shader after its own preamble, where NVIDIA refuses an #extension line (without
  # it the shader runs in full precision); and the last pass, which writes luma only, must say
  # one component, or merging it with chroma fails an assertion that takes the app down.
  curl -fsSL "https://raw.githubusercontent.com/Artoriuz/ArtCNN/$artcnn/GLSL/ArtCNN_$name.glsl" |
    grep -v '^#extension GL_EXT_shader_explicit_arithmetic_types_float16' |
    awk '/\(Depth-To-Space\)/ { last = 1 }
         last && /^\/\/!COMPONENTS 4$/ { print "//!COMPONENTS 1"; last = 0; next }
         { print }' >"$dir/ArtCNN_$name.glsl"
done

echo "shaders in $dir"
