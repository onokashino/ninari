---
title: Releasing
description: How to release a new ninari version: a tag, the build on GitHub and the update for users.
section: dev
---
# Releasing

1. Raise `version` in `crates/app/Cargo.toml` and commit.
2. Tag it with the note users will see, and push:

```bash
git tag -a v0.2.0 -m "What's new, as the app will show it"
git push origin master v0.2.0
```

The `.github/workflows/release.yml` workflow builds the Linux archive in an Arch Linux container (`scripts/package.sh`) and the Windows zip and installer (`scripts/package.ps1`, Inno Setup), then publishes the release with `latest.json`:

```json
{
  "version": "0.2.0",
  "notes": "the tag's message",
  "platforms": {
    "linux-x86_64": { "url": "…/ninari-0.2.0-linux-x86_64.tar.gz", "sha256": "…" },
    "windows-x86_64": { "url": "…/ninari-0.2.0-windows-x86_64-setup.exe", "sha256": "…" }
  }
}
```

Installed apps read that file, offer the update, download their system's package and check its checksum. On Linux they unpack the archive and run its `install.sh`, on Windows they run the installer quietly. Then the app restarts.

Every push to master that touches the app builds both systems too: the packages are kept as the run's artifacts, no release is made.

The tag must match the version in `Cargo.toml`, or the build stops. The Shikimori keys come from the repository's `SHIKIMORI_CLIENT_ID` and `SHIKIMORI_CLIENT_SECRET` secrets.
