---
title: Your own source
description: How to write an episode source plugin for ninari in TypeScript, test it and add it.
section: dev
---
# Your own source

A plugin is one TypeScript file. It exports a manifest and three functions: find a title, list its episodes and return a stream link. The app picks the best quality, merges dubs and plays the video.

## Getting set up

Plugins live in the repository's `extensions` folder. You'll need [bun](https://bun.sh).

```bash
git clone https://github.com/onokashino/ninari
cd ninari/extensions
```

Create `src/mysite.ts`. The types are in `src/types.ts`.

## The manifest

```ts
import type { Ctx, EpisodeList, Manifest, Match, SearchQuery, Stream } from "./types";

export const manifest: Manifest = {
  id: "mysite",
  name: "My site",
  version: "0.1.0",
  apiVersion: 1,
  // Main domain first, then mirrors: ctx.baseUrl points at the one that works.
  domains: ["https://mysite.example", "https://mirror.mysite.example"],
  // Other hosts it may reach, like a CDN: an exact name or "*.suffix".
  allowedHosts: ["*.cdn.mysite.example"],
};
```

The plugin can only reach these addresses, and users see them before installing.

## search: find the title

The app passes the Shikimori and MyAnimeList ids, names in several languages, the year, the kind and the episode count. Return what you found: `exact` when you're sure (the id matched), otherwise `fuzzy`.

```ts
export async function search(q: SearchQuery, ctx: Ctx): Promise<Match[]> {
  const res = await ctx.fetch(`${ctx.baseUrl}/api/search?q=${encodeURIComponent(q.names[0])}`);
  if (!res.ok) return [];
  const items = (await res.json()) as { id: number; title: string; shikimori_id?: number }[];
  return items.map((it) => ({
    id: String(it.id),
    title: it.title,
    confidence: it.shikimori_id === q.shikimoriId ? "exact" : "fuzzy",
  }));
}
```

## episodes: episodes and dubs

```ts
export async function episodes(m: Match, ctx: Ctx): Promise<EpisodeList> {
  const res = await ctx.fetch(`${ctx.baseUrl}/api/anime/${m.id}`);
  if (res.status === 404) ctx.fail.notFound(`anime ${m.id}`);
  const anime = (await res.json()) as { episodes: { n: number; hd: boolean }[] };
  return {
    dubs: [{ id: "main", name: "My site", type: "voice" }],
    episodes: anime.episodes.map((e) => ({
      number: e.n,
      available: [{ dubId: "main", maxQuality: e.hd ? 1080 : 720 }],
      // Optional: where the opening and ending are, in seconds.
      skip: { opening: [90, 180] },
    })),
  };
}
```

A dub's `type` is `voice`, `subtitles` or `original`. `maxQuality` is shown next to the dub and helps pick the best one.

## resolve: the stream link

```ts
export async function resolve(m: Match, episode: number, dubId: string, ctx: Ctx): Promise<Stream[]> {
  const res = await ctx.fetch(`${ctx.baseUrl}/api/anime/${m.id}/episode/${episode}`);
  const data = (await res.json()) as { hls: string; quality: number };
  return [
    {
      url: data.hls,
      kind: "hls", // or "dash", "mp4"
      quality: data.quality,
      // Headers the site insists on: the local proxy adds them.
      headers: { Referer: `${ctx.baseUrl}/` },
    },
  ];
}
```

You may return several streams of different quality: the app starts with the best.

## What ctx offers

| What | For |
| --- | --- |
| `ctx.baseUrl` | The working domain from `domains` |
| `ctx.fetch(url, init)` | A request: `method`, `headers`, `body`, `timeout` in ms |
| `ctx.html.select(html, css)` | Parse HTML: nodes with `text`, `html`, `attrs` |
| `ctx.kv.get` / `ctx.kv.set(key, value, ttlSec)` | The plugin's cache, say for a token |
| `ctx.base64` | `encode` and `decode` |
| `ctx.fail.notFound(msg)` | No such title or episode |
| `ctx.fail.geoBlocked(msg)` | Closed in this region |
| `ctx.fail.challenge(url)` | The site wants a check passed |
| `ctx.log(...)` | A line in the app's log |

`ctx.fail.*` end the call and tell the app why: it shows the reason in the sources' settings and takes the episode from another source.

## The same dub on different sites

So that one dub team from different sites shows as one line, its names are merged in `dubs.json`: a key and the spellings, normalised (lower case, letters and digits only).

```json
{
  "anidub": { "name": "AniDUB", "aliases": ["anidub", "anidubonline", "анидаб"] }
}
```

## Testing

Put `src/mysite.test.ts` next to the plugin: tests reach the live site through `makeCtx` from `src/testing/host.ts`.

```ts
import { expect, test } from "bun:test";
import * as ext from "./mysite";
import { makeCtx } from "./testing/host";

const ctx = makeCtx(ext.manifest.domains[0]);

test("finds Frieren", async () => {
  const found = await ext.search(
    { shikimoriId: 52991, malId: 52991, names: ["Sousou no Frieren"], year: 2023 },
    ctx,
  );
  expect(found.length).toBeGreaterThan(0);
}, 30_000);
```

```bash
bun test src/mysite.test.ts
```

## Build and add it

```bash
bun run build
```

The build writes `dist/mysite.js` and updates `dist/index.json` with checksums. To try it yourself, in ninari open **Настройки → Источники → Свой репозиторий** (Settings → Sources → Own repository) and give the path to `extensions/dist/index.json`.

## Share it

Host the `dist` folder anywhere over https and share a link like:

```text
ninari://plugins?repo=https://example.org/dist/index.json
```

It opens ninari and offers the plugins once the user confirms. Want it in the official catalogue? Send a [pull request](https://github.com/onokashino/ninari/pulls) with the plugin and its test.
