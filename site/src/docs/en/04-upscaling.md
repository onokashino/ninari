---
title: Upscaling
description: Anime4K and ArtCNN in ninari: what they do, which mode to pick and what they need from the graphics card.
section: guide
---
# Upscaling

ninari can redraw the picture with neural networks when the video is smaller than the screen, say 720p on a 1440p monitor. Upscaling turns on by itself when it makes sense and is tuned right while watching: the sparkle icon in the player opens its panel.

## Two methods

**Anime4K** restores and enlarges the picture in several passes. It has modes:

| Mode | When |
| --- | --- |
| Auto (Авто) | Picks by the video's height. Recommended |
| A | Blurry and heavily compressed sources: 480p, 720p |
| B | The same, gentler, fewer halos around lines |
| C | Clean sources: only enlarges, leaving detail alone |

**ArtCNN** is kinder to lines and adds no artefacts, but only doubles the size, so it works when the screen is clearly bigger than the video. Flavours: **Обычный** (neutral), **Чётче** (sharper) and **Мягче** (softer).

## Load

| Level | For |
| --- | --- |
| Fast | Most graphics cards, integrated ones included |
| HQ | A better picture; needs a discrete card like a GTX 1660 or better |
| Ultra | The biggest networks, for RTX 3070 class cards and up |

If the video stutters, lower the level in the panel. The default level is set in **Настройки → Плеер** (Settings → Player), where upscaling can also be turned off.

## Compare

Hold <kbd>C</kbd>: while it's down you see the original. <kbd>U</kbd> turns upscaling on and off altogether.
