---
title: Plugins
description: How ninari's plugins find episodes, how to add and update them, and how to use a third-party catalogue.
section: guide
---
# Plugins

ninari keeps and serves no video. Episodes are found by plugins: each knows one site, looks the title up there and hands the player links to episodes.

## Adding them

**Настройки → Источники → Подключить все** (Settings → Sources → Add all) installs the plugins from the official catalogue. The same page shows how each source did over the week, and **Обновить все** (update all) installs new versions.

The plugins and what they do are listed on the [Plugins](../plugins.html) page.

## Safety

- Each plugin runs in a sandbox with no access to files or the system.
- It can only reach the sites on its card; the list is shown before installing.
- A plugin's file is checked against the catalogue's checksum, so a swapped file won't install.

## A third-party catalogue

A catalogue is an `index.json` file listing plugins. There are two ways to add someone else's:

- **Настройки → Источники → Свой репозиторий** (Settings → Sources → Own repository): paste a link to `index.json` or a path to it on disk.
- A link like `ninari://plugins?repo=<address of index.json>` on a website opens the app.

Either way, ninari first shows the plugins and the sites they reach, and installs them only once you confirm.

Making your own plugin is not hard: see [Your own source](custom-source.html).

## When a source finds the wrong title

A title's page has **Не тот тайтл?** (wrong title?). It shows what each source found and lets you pick the right one, search by hand, or mark that the source doesn't have the title.
