# Downloads the player components bundled with ninari:
# mpv (shinchiro build), Anime4K and ArtCNN shaders. The app draws its own player controls.
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$root = Split-Path -Parent $PSScriptRoot
$app = Join-Path $root "crates\app"
$bin = Join-Path $app "binaries"
$cfg = Join-Path $app "mpv-config"
$tmp = Join-Path ([IO.Path]::GetTempPath()) "ninari-fetch"
New-Item -ItemType Directory -Force $bin, $cfg, $tmp | Out-Null
# System tar (libarchive) can unpack both zip and 7z.
$tar = Join-Path $env:SystemRoot "System32\tar.exe"
$headers = @{ "User-Agent" = "ninari-fetch" }

function Get-Asset([string]$repo, [string]$pattern) {
  $rel = Invoke-RestMethod -Headers $headers "https://api.github.com/repos/$repo/releases/latest"
  $asset = $rel.assets | Where-Object { $_.name -match $pattern } | Select-Object -First 1
  if (-not $asset) { throw "no asset matching $pattern in $repo" }
  $out = Join-Path $tmp $asset.name
  if (-not (Test-Path $out)) {
    Write-Host "downloading $($asset.name)"
    Invoke-WebRequest -Headers $headers $asset.browser_download_url -OutFile $out
  }
  return $out
}

# mpv: the generic x86_64 build, not "-v3" which needs AVX2.
$archive = Get-Asset "shinchiro/mpv-winbuild-cmake" '^mpv-x86_64-\d{8}-git-[0-9a-f]+\.7z$'
$mpvDir = Join-Path $tmp "mpv"
Remove-Item -Recurse -Force $mpvDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory $mpvDir | Out-Null
& $tar -xf $archive -C $mpvDir
Copy-Item (Join-Path $mpvDir "mpv.exe") (Join-Path $bin "mpv-x86_64-pc-windows-msvc.exe") -Force

# Anime4K GLSL shaders.
$a4k = Get-Asset "bloc97/Anime4K" '^Anime4K_v4\.0\.zip$'
$shaders = Join-Path $cfg "shaders"
New-Item -ItemType Directory -Force $shaders | Out-Null
& $tar -xf $a4k -C $shaders

# ArtCNN GLSL shaders, pinned, with the two fixes scripts/fetch-shaders.sh explains: no
# float16 #extension line, and one component for the last pass, which writes luma only.
$artcnn = "0837e5326e710f49e2ab0a8d995da0708047442e"
foreach ($name in "C4F16", "C4F16_DS", "C4F16_DN", "C4F32", "C4F32_DS", "C4F32_DN") {
  $url = "https://raw.githubusercontent.com/Artoriuz/ArtCNN/$artcnn/GLSL/ArtCNN_$name.glsl"
  $text = (Invoke-WebRequest -Headers $headers $url).Content
  $lines = $text -split "`n" | Where-Object { $_ -notmatch '^#extension GL_EXT_shader_explicit_arithmetic_types_float16' }
  $last = $false
  $lines = foreach ($line in $lines) {
    if ($line -match '\(Depth-To-Space\)') { $last = $true }
    if ($last -and $line -eq '//!COMPONENTS 4') { $last = $false; '//!COMPONENTS 1' } else { $line }
  }
  $text = $lines -join "`n"
  Set-Content -NoNewline -Encoding utf8 (Join-Path $shaders "ArtCNN_$name.glsl") $text
}

Write-Host "done: $bin, $cfg"
