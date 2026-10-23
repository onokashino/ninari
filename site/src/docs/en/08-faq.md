---
title: Questions and problems
description: Common questions about ninari and what to do when something doesn't work.
section: guide
---
# Questions and problems

## A title doesn't have the dub I want

None of your sources has it. Check that plugins are added: **Настройки → Источники** (Settings → Sources). If a source found the wrong title, open **Не тот тайтл?** (wrong title?) on the title's page and pick the right one.

## The franchise's seasons don't show

Seasons come from Shikimori. When it limits requests for a while, the app waits and asks again. If the row still doesn't appear, open the title again later.

## No fillers or episode titles

They come from MyAnimeList. If the Jikan service is out of reach from your network, ninari reads MyAnimeList's own pages; the first time a title opens may take a few seconds.

## The video doesn't start

- Try another dub or source: the site may be broken. The sources' health is in **Настройки → Источники** (Settings → Sources).
- If that doesn't help, [report it](https://github.com/onokashino/ninari/issues) with the version from **Настройки → Приложение** (Settings → App).

## The picture stutters

Lower the upscaling level in the player's panel or turn it off with <kbd>U</kbd>. See [Upscaling](upscaling.html).

## The window closed but the app is still running

A closed window goes to the tray and keeps checking for new episodes. Quit from the tray icon's menu. This is set in **Настройки → Приложение → Сворачивать в трей при закрытии** (Settings → App → Hide to tray on close).

## Where is my data

The library and settings are in `~/.local/share/com.onokashino.ninari`, posters and previews in `~/.cache/com.onokashino.ninari`. The cache can be cleared in **Настройки → Данные** (Settings → Data).

## Is it free

Yes, with no ads or subscriptions. The code is open on [GitHub](https://github.com/onokashino/ninari).
