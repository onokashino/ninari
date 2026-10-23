---
title: Свой источник
description: Как написать плагин-источник для ninari на TypeScript, проверить его и подключить.
section: dev
---
# Свой источник

Плагин это один файл на TypeScript. Он экспортирует манифест и три функции: найти тайтл, перечислить серии и отдать ссылку на поток. Приложение само выберет лучшее качество, сведёт озвучки и сыграет видео.

## Подготовка

Плагины живут в папке `extensions` репозитория. Понадобится [bun](https://bun.sh).

```bash
git clone https://github.com/onokashino/ninari
cd ninari/extensions
```

Создай `src/mysite.ts`. Типы лежат в `src/types.ts`.

## Манифест

```ts
import type { Ctx, EpisodeList, Manifest, Match, SearchQuery, Stream } from "./types";

export const manifest: Manifest = {
  id: "mysite",
  name: "Мой сайт",
  version: "0.1.0",
  apiVersion: 1,
  // Основной домен первым, затем зеркала: ctx.baseUrl укажет на рабочий.
  domains: ["https://mysite.example", "https://mirror.mysite.example"],
  // Хосты, куда ещё можно ходить, например CDN: точное имя или "*.суффикс".
  allowedHosts: ["*.cdn.mysite.example"],
};
```

Плагин может ходить в сеть только на эти адреса. Пользователь увидит их перед установкой.

## search: найти тайтл

Приложение передаёт id Shikimori и MyAnimeList, названия на разных языках, год, тип и число серий. Верни найденные варианты: `exact`, если уверен (совпал id), иначе `fuzzy`.

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

## episodes: серии и озвучки

```ts
export async function episodes(m: Match, ctx: Ctx): Promise<EpisodeList> {
  const res = await ctx.fetch(`${ctx.baseUrl}/api/anime/${m.id}`);
  if (res.status === 404) ctx.fail.notFound(`anime ${m.id}`);
  const anime = (await res.json()) as { episodes: { n: number; hd: boolean }[] };
  return {
    dubs: [{ id: "main", name: "Мой сайт", type: "voice" }],
    episodes: anime.episodes.map((e) => ({
      number: e.n,
      available: [{ dubId: "main", maxQuality: e.hd ? 1080 : 720 }],
      // Необязательно: где опенинг и эндинг, в секундах.
      skip: { opening: [90, 180] },
    })),
  };
}
```

`type` озвучки: `voice`, `subtitles` или `original`. `maxQuality` показывается рядом с озвучкой и помогает выбрать лучшую.

## resolve: ссылка на поток

```ts
export async function resolve(m: Match, episode: number, dubId: string, ctx: Ctx): Promise<Stream[]> {
  const res = await ctx.fetch(`${ctx.baseUrl}/api/anime/${m.id}/episode/${episode}`);
  const data = (await res.json()) as { hls: string; quality: number };
  return [
    {
      url: data.hls,
      kind: "hls", // или "dash", "mp4"
      quality: data.quality,
      // Заголовки, без которых сайт не отдаёт поток: их добавит локальный прокси.
      headers: { Referer: `${ctx.baseUrl}/` },
    },
  ];
}
```

Можно вернуть несколько потоков разного качества: приложение начнёт с лучшего.

## Что есть в ctx

| Что | Для чего |
| --- | --- |
| `ctx.baseUrl` | Рабочий домен из `domains` |
| `ctx.fetch(url, init)` | Запрос: `method`, `headers`, `body`, `timeout` в мс |
| `ctx.html.select(html, css)` | Разобрать HTML: узлы с `text`, `html`, `attrs` |
| `ctx.kv.get` / `ctx.kv.set(key, value, ttlSec)` | Кэш плагина, например токен |
| `ctx.base64` | `encode` и `decode` |
| `ctx.fail.notFound(msg)` | Тайтла или серии нет |
| `ctx.fail.geoBlocked(msg)` | Закрыто в регионе |
| `ctx.fail.challenge(url)` | Сайт требует пройти проверку |
| `ctx.log(...)` | Строка в журнал приложения |

`ctx.fail.*` прерывают вызов и сообщают приложению причину: оно покажет её в настройках источников и возьмёт серию из другого.

## Озвучки с разных сайтов

Чтобы одна и та же команда с разных сайтов шла одной строкой, её названия сводятся в `dubs.json`: ключ и варианты написания в нормализованном виде (нижний регистр, только буквы и цифры).

```json
{
  "anidub": { "name": "AniDUB", "aliases": ["anidub", "anidubonline", "анидаб"] }
}
```

## Проверить

Рядом с плагином положи `src/mysite.test.ts`: тесты ходят на живой сайт через `makeCtx` из `src/testing/host.ts`.

```ts
import { expect, test } from "bun:test";
import * as ext from "./mysite";
import { makeCtx } from "./testing/host";

const ctx = makeCtx(ext.manifest.domains[0]);

test("находит Фрирен", async () => {
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

## Собрать и подключить

```bash
bun run build
```

Сборка кладёт `dist/mysite.js` и обновляет `dist/index.json` с контрольными суммами. Чтобы попробовать у себя, в ninari открой **Настройки → Источники → Свой репозиторий** и укажи путь к `extensions/dist/index.json`.

## Поделиться

Выложи папку `dist` на любой хостинг по https и дай ссылку вида:

```text
ninari://plugins?repo=https://example.org/dist/index.json
```

Она откроет ninari и предложит установить плагины после подтверждения. Хочешь в официальный каталог? Пришли [pull request](https://github.com/onokashino/ninari/pulls) с плагином и тестом.
