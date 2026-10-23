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

The `.github/workflows/release.yml` workflow builds the archive in an Arch Linux container (`scripts/package.sh`) and publishes the release with `latest.json`. Installed apps read that file and offer the update.

The tag must match the version in `Cargo.toml`, or the build stops. The Shikimori keys come from the repository's `SHIKIMORI_CLIENT_ID` and `SHIKIMORI_CLIENT_SECRET` secrets.
