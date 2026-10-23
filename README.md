# ninari

A desktop app for watching anime: the Shikimori catalogue, episodes from source plugins, and a
player with Anime4K and ArtCNN upscaling. Written in Rust; the window is drawn with GPUI (the UI
framework of the Zed editor) and video plays through libmpv.

## Layout

- `crates/core`: everything but the window: the catalogue, the library, sources and their
  plugins, the player's controller (mpv), devices on the local network, Shikimori sign-in.
- `crates/app`: the app: its screens, the player, the tray and `ninari://` links, and `shot`,
  which draws a page to a PNG without opening a window.
- `extensions`: the source plugins (TypeScript, built with bun into `extensions/dist`, which the
  app installs from); see its README.
- `site`: the website and the docs, in Russian and English: `bun site/build.ts` builds them from
  `site/src` into `site/dist`, and `.github/workflows/site.yml` publishes them on GitHub Pages.

## Running on Linux

Needs Rust, mpv with libmpv, and bun for the short commands below.

    bun shaders                # once: the upscaling shaders (scripts/fetch-shaders.sh)
    bun gpui                   # build and start; --page title:<id> opens a title
    bun play                   # straight into the episode to continue
    bun shot out.png catalog   # draw a page to a PNG, no window

`scripts/dev` runs everything under a memory cap with no swap.

Signing in to Shikimori needs the app's keys at build time: `.env.local` at the project root
with `SHIKIMORI_CLIENT_ID=...` and `SHIKIMORI_CLIENT_SECRET=...` lines. It is git-ignored.

## Installing a release

Download `ninari-<version>-linux-x86_64.tar.gz` from GitHub Releases, unpack it and run
`./install.sh` inside. It copies the app into `~/.local/share/ninari`, puts `ninari` into
`~/.local/bin` and an entry into the app menu; running a newer release's `install.sh` updates it,
`install.sh --remove` takes it away (the library stays). The release carries its own mpv,
libmpv and the libraries they need; only glibc, the GPU driver, X11/Wayland, the sound server and
glib come from the system. It is built on Arch Linux and needs a distribution as current.

## Releasing

Raise `version` in `crates/app/Cargo.toml`, commit, then push a matching tag:

    git tag -a v0.2.0 -m "What is new, as the app shows it"
    git push origin master v0.2.0

`.github/workflows/release.yml` builds the archive in an Arch Linux container
(`scripts/package.sh`) and publishes it with `latest.json`, from which installed apps learn about
the version. The repository's secrets `SHIKIMORI_CLIENT_ID` and `SHIKIMORI_CLIENT_SECRET` go into
the build.

## Windows

`scripts/fetch-mpv.ps1` downloads mpv and the shaders. Video inside the window works on Linux
only for now (it draws into a Wayland subsurface); on Windows mpv plays in a window of its own.
