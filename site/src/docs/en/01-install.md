---
title: Installing
description: How to install ninari on Linux with one command, update it and remove it.
section: guide
---
# Installing

ninari currently ships for Linux. The archive is built on Arch and needs current system libraries (glibc 2.44 or newer): Arch, openSUSE Tumbleweed and similar distributions work. A Flatpak for everything else and an AUR package for Arch are coming; Windows and macOS versions are in progress.

## With one command

```bash
curl -L https://github.com/onokashino/ninari/releases/download/v0.1.0/ninari-0.1.0-linux-x86_64.tar.gz | tar xz && ./ninari-*/install.sh
```

The command downloads the latest release, unpacks it into the current folder and runs the installer. The current command is always on the [Download](../download.html) page.

## By hand

1. Download `ninari-<version>-linux-x86_64.tar.gz` from the [releases page](https://github.com/onokashino/ninari/releases).
2. Unpack it and run `./install.sh` inside.
3. ninari shows up in your app menu and as the `ninari` command.

No password is needed: everything goes into your home folder.

## What goes where

| What | Where |
| --- | --- |
| The program, its own mpv and libraries | `~/.local/share/ninari` |
| The `ninari` command | `~/.local/bin/ninari` |
| Menu entry and icons | `~/.local/share/applications`, `~/.local/share/icons` |
| Library, progress, settings | `~/.local/share/com.onokashino.ninari` |
| Posters and previews | `~/.cache/com.onokashino.ninari` |

The archive carries mpv and every library it needs. The system's mpv is neither needed nor touched; only glibc, the graphics driver, X11 or Wayland and sound come from the system.

## Updating

When a new version is out, ninari says so in a corner of its window. Download the new archive and run its `install.sh`: the old version is replaced, your library and settings stay.

## Removing

```bash
./install.sh --remove
```

This removes the program, the command, the menu entry and the icons. Your library and settings in `~/.local/share/com.onokashino.ninari` stay: delete that folder yourself if you no longer need it.
