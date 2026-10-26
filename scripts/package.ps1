# Packs the Windows release into dist/: a folder with everything the app needs beside the program
# (mpv.exe, mpv-config with the shaders), as a zip and as an installer.
#
#   pwsh scripts/package.ps1
#
# Needs Rust, and Inno Setup's iscc for the installer (the zip is made without it).
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$root = Split-Path -Parent $PSScriptRoot
$app = Join-Path $root "crates\app"
$env:CARGO_TARGET_DIR = Join-Path $root "target"
$version = (Select-String -Path (Join-Path $app "Cargo.toml") -Pattern '^version = "(.*)"').Matches[0].Groups[1].Value
$name = "ninari-$version-windows-x86_64"
$dist = Join-Path $root "dist"
$out = Join-Path $dist $name

# mpv and the shaders, unless already there.
if (-not (Test-Path (Join-Path $app "binaries\mpv-x86_64-pc-windows-msvc.exe")) -or
    -not (Test-Path (Join-Path $app "mpv-config\shaders"))) {
  & (Join-Path $PSScriptRoot "fetch-mpv.ps1")
}

Push-Location $app
try {
  cargo build --release --bin ninari
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally {
  Pop-Location
}

Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item (Join-Path $env:CARGO_TARGET_DIR "release\ninari.exe") $out
Copy-Item (Join-Path $app "binaries\mpv-x86_64-pc-windows-msvc.exe") (Join-Path $out "mpv.exe")
Copy-Item -Recurse (Join-Path $app "mpv-config") (Join-Path $out "mpv-config")
Copy-Item (Join-Path $root "LICENSE") (Join-Path $out "LICENSE.txt")

$zip = Join-Path $dist "$name.zip"
Remove-Item -Force $zip -ErrorAction SilentlyContinue
Compress-Archive -Path $out -DestinationPath $zip
Write-Host "packed $zip"

$iscc = Get-Command iscc -ErrorAction SilentlyContinue
if (-not $iscc) {
  $iscc = Get-Item "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" -ErrorAction SilentlyContinue
}
if ($iscc) {
  & $iscc.Source /Qp "/DVersion=$version" "/DSource=$out" "/DOutput=$dist" "/DIcon=$(Join-Path $app 'icons\icon.ico')" (Join-Path $PSScriptRoot "ninari.iss")
  if ($LASTEXITCODE -ne 0) { throw "iscc failed" }
  Write-Host "packed $(Join-Path $dist "$name-setup.exe")"
} else {
  Write-Host "no Inno Setup: the installer is skipped"
}
