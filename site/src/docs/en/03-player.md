---
title: Player
description: ninari's player controls, keyboard shortcuts, skips, sound and quality.
section: guide
---
# Player

ninari plays through mpv. On Wayland the video plays inside the app's window; on X11 it opens in an mpv window of its own.

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| <kbd>Space</kbd> or <kbd>K</kbd> | Pause |
| <kbd>←</kbd> <kbd>→</kbd> | Back and forward 10 seconds |
| <kbd>↑</kbd> <kbd>↓</kbd> | Volume |
| <kbd>F</kbd> or double click | Fullscreen |
| <kbd>M</kbd> | Mute |
| <kbd>N</kbd> <kbd>P</kbd> | Next and previous episode |
| <kbd>U</kbd> | Upscaling on or off |
| Hold <kbd>C</kbd> | Show the original without upscaling |
| <kbd>Z</kbd> | Fill the screen or fit |
| <kbd>[</kbd> <kbd>]</kbd> | Shift the sound against the picture |
| <kbd>Esc</kbd> | Close a panel, leave fullscreen, close the player |

## Skips and the next episode

The player settings have **Пропускать опенинги** (skip openings), **Пропускать эндинги** (skip endings) and **Пропускать филлеры** (skip fillers). When a source knows where the opening is and auto-skip is off, a **Пропустить опенинг** button shows during it.

The next episode appears over the credits. It follows the story: fillers and recaps, as MyAnimeList marks them, can be passed over.

**Продолжить** (continue) starts 5 seconds early so you know where you were.

## Sound

Each effect switches on by itself:

- **Ровная громкость** (even loudness): quiet lines louder, explosions quieter.
- **Чёткие голоса** (clear voices): voices forward, music and noise in the background.
- **Ночной режим** (night mode): loud moments don't blast.
- **Шумодав** (denoise): removes the hiss and hum of home-recorded dubs.
- **Наушники** (headphones): a softer stereo spread.

## Dub and quality

You can switch the dub right in the player and keep your place. The highest quality is picked by itself; the quality menu can lower it.

Seeking shows frames over the bar. They are gathered while you watch and cached; the cache can be cleared in **Настройки → Данные** (Settings → Data).
