---
title: How it's built
description: What ninari is made of: the core, the GPUI app, plugins and the site.
section: dev
---
# How it's built

ninari is written in Rust. GPUI, the UI framework of the Zed editor, draws the window; video plays through libmpv.

| Folder | What's inside |
| --- | --- |
| `crates/core` | Everything but the window: the Shikimori catalogue, the library, sources and plugins, the player (mpv control), the stream proxy, device sync, Shikimori sign-in |
| `crates/app` | The app: its screens, the player, the tray, `ninari://` links, and `shot` to draw pages to PNG |
| `extensions` | The official TypeScript plugins and their build into `dist/` |
| `site` | This site: pages, docs and the builder |
| `scripts` | Running, packing a release, the installer, shaders, icons |

## How an episode reaches the screen

1. The catalogue, title details and posters come from Shikimori; banners and main characters from AniList.
2. Each plugin looks the title up on its site (`search`) and returns the episodes with their dubs (`episodes`).
3. The core merges the same dub from different sites into one line using `dubs.json`.
4. When an episode starts, the plugin returns stream links (`resolve`), a local proxy adds the headers the site wants, and mpv plays.
5. Progress goes to SQLite and, when enabled, to Shikimori and to paired devices.

## The plugin sandbox

Plugins run in QuickJS inside the app. The network is reachable only through `ctx.fetch` and only to the manifest's domains; private addresses are closed.
