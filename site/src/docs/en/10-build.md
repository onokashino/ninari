---
title: Building from source
description: How to build and run ninari from source on Linux.
section: dev
---
# Building from source

## What you need

- Rust (stable).
- mpv with libmpv from your system's packages.
- bun, for the short commands and the plugins.

## Running

```bash
git clone https://github.com/onokashino/ninari
cd ninari
bun shaders          # once: the upscaling shaders
bun gpui             # build and start
```

| Command | What it does |
| --- | --- |
| `bun gpui` | Build and start. `--page title:<id>` opens a title |
| `bun play` | Straight into the episode on the continue card |
| `bun shot out.png catalog` | Draw a page to a PNG without a window |
| `bun plugins` | Build the plugins into `extensions/dist` |
| `bun plugins:test` | Test the plugins against the live sources |

`scripts/dev` runs everything under a memory cap with no swap.

## Tests

```bash
cd crates/core && cargo test
cd crates/app && cargo test
```

## Shikimori keys

Signing in to Shikimori needs the app's keys at build time. Create `.env.local` at the project root:

```bash
SHIKIMORI_CLIENT_ID=...
SHIKIMORI_CLIENT_SECRET=...
```

The file is git-ignored. The Shikimori application is registered with the redirect URI `http://127.0.0.1:47831/oauth/shikimori` and the `user_rates` scope.
