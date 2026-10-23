import { describe, expect, test } from "bun:test";
import * as ext from "./rutube";
import { makeCtx } from "./testing/host";

const video = (id: string, season: number, episode: number, title: string, author = "РуАниме", authorId = 27141608) => ({
  id,
  season,
  episode,
  title,
  author: { id: authorId, name: author },
});

describe("rutube: which season of the show a Shikimori title is", () => {
  test("from the romaji, the English or the trailing number of the Russian name", () => {
    expect(ext.seasonOf(["Shen Yin Wangzuo 2nd Season", "Трон, отмеченный богом 2", "神印王座 第二季"])).toEqual({
      season: 2,
      names: ["Shen Yin Wangzuo", "Трон, отмеченный богом", "神印王座"],
    });
    expect(ext.seasonOf(["Wan Jie Du Zun Season 3"])?.season).toBe(3);
    expect(ext.seasonOf(["Покров тьмы 4 сезон"])?.season).toBe(4);
  });

  test("a first season has no marker", () => {
    expect(ext.seasonOf(["Shen Yin Wangzuo", "Трон, отмеченный богом"])).toEqual({
      season: 1,
      names: ["Shen Yin Wangzuo", "Трон, отмеченный богом"],
    });
  });

  test("a season split into parts is not guessed at", () => {
    expect(ext.seasonOf(["Wanmei Shijie 2nd Season Part 2"])).toBeNull();
    expect(ext.seasonOf(["Идеальный мир 2: Часть 2"])).toBeNull();
  });
});

test("a show's catalogue name matches when one of its parts is the title's name", () => {
  const feed = "Трон, отмеченный Богом / Shen Yin Wang Zuo / Sealed Divine Throne";
  expect(ext.feedMatches(feed, ["Трон, отмеченный богом"])).toBe(true);
  expect(ext.feedMatches(feed, ["Shen Yin Wangzuo"])).toBe(true);
  expect(ext.feedMatches(feed, ["Трон"])).toBe(false);
  expect(ext.feedMatches("Боевой континент / Douluo Dalu", ["Трон, отмеченный богом"])).toBe(false);
});

describe("rutube: episode numbers", () => {
  // The team numbers the show through all its seasons: "2 сезон, 16 серия (42 серия)". Season 4 is
  // missing on Rutube; a Shikimori "2nd Season" counts from the first episode of season 2.
  const videos = [
    video("trailer", 0, 0, "Трон, отмеченный Богом (трейлер)"),
    video("s2e1", 2, 1, "Трон, отмеченный Богом, 2 сезон, 1 серия (27 серия)"),
    video("s2e16", 2, 16, "Трон, отмеченный Богом, 2 сезон, 16 серия (42 серия)"),
    video("s3e16", 3, 16, "Трон, отмеченный Богом, 3 сезон, 16 серия (68 серия)"),
    video("s5e1", 5, 1, "Трон, отмеченный Богом, 5 сезон, 1 серия (105 серия)"),
  ];

  test("run on through later seasons by the show-wide number, gaps and all", () => {
    const map = ext.episodeMap(videos, 2, 182);
    expect(map.get("27141608")).toEqual(
      new Map([
        [1, "s2e1"],
        [16, "s2e16"],
        [42, "s3e16"],
        [79, "s5e1"],
      ]),
    );
  });

  test("stop at the title's episode count, where the next Shikimori season begins", () => {
    expect([...ext.episodeMap(videos, 2, 26).get("27141608")!.keys()]).toEqual([1, 16]);
  });

  test("start where the title's season starts even if its first episode is not uploaded", () => {
    const map = ext.episodeMap(videos.filter((v) => v.id !== "s2e1"), 2, 0);
    expect(map.get("27141608")?.get(42)).toBe("s3e16");
  });

  test("without show-wide numbers only the season itself is used", () => {
    const plain = [video("a1", 1, 1, "Серия 1"), video("a2", 1, 2, "Серия 2"), video("b1", 2, 1, "Серия 1")];
    expect(ext.episodeMap(plain, 1, 0).get("27141608")).toEqual(
      new Map([
        [1, "a1"],
        [2, "a2"],
      ]),
    );
  });

  test("each uploader is its own dub", () => {
    const two = [video("x", 2, 1, "2 сезон, 1 серия (27 серия)"), video("y", 2, 1, "Серия 1", "AnimeVost", 5)];
    const map = ext.episodeMap(two, 2, 0);
    expect(map.get("27141608")?.get(1)).toBe("x");
    expect(map.get("5")?.get(1)).toBe("y");
  });
});

test("a master playlist becomes one stream per picture size, best first, each size once", () => {
  const master = [
    "#EXTM3U",
    '#EXT-X-STREAM-INF:BANDWIDTH=980000,CODECS="avc1.42c01f, mp4a.40.2",RESOLUTION=432x232',
    "https://river-1.rutube.ru/a/232.m3u8",
    '#EXT-X-STREAM-INF:BANDWIDTH=4900000,CODECS="avc1.42c01f, mp4a.40.2",RESOLUTION=1920x1080',
    "https://river-ntv.rtbcdn.ru/a/1080.m3u8",
    '#EXT-X-STREAM-INF:BANDWIDTH=4900000,CODECS="avc1.42c01f, mp4a.40.2",RESOLUTION=1920x1080',
    "https://river-1.rutube.ru/a/1080.m3u8",
    '#EXT-X-STREAM-INF:BANDWIDTH=2900000,RESOLUTION=1280x720',
    "https://river-1.rutube.ru/a/720.m3u8",
    '#EXT-X-STREAM-INF:BANDWIDTH=2400000,RESOLUTION=1920x800',
    "https://river-1.rutube.ru/a/wide.m3u8",
  ].join("\n");
  expect(ext.variants(master).map((v) => [v.quality, v.url])).toEqual([
    [1080, "https://river-ntv.rtbcdn.ru/a/1080.m3u8"],
    [720, "https://river-1.rutube.ru/a/720.m3u8"],
    [232, "https://river-1.rutube.ru/a/232.m3u8"],
  ]);
});

describe("rutube (live)", () => {
  const ctx = makeCtx(ext.manifest.domains[0]);
  const throne2 = {
    shikimoriId: 52684,
    malId: 52684,
    names: ["Трон, отмеченный богом 2", "Shen Yin Wangzuo 2nd Season", "神印王座 第二季"],
    year: 2022,
    kind: "ona",
    episodes: 182,
  };

  test("finds the show's catalogue by its Russian name", async () => {
    const [m] = await ext.search(throne2, ctx);
    expect(m).toMatchObject({ id: "262595", confidence: "exact" });
  }, 60_000);

  test("Shikimori episode 42 of the 2nd season is the team's episode 68, in 1080p", async () => {
    const [m] = await ext.search(throne2, ctx);
    const list = await ext.episodes(m, ctx);
    const team = list.dubs.find((d) => d.name === "РуАниме");
    expect(team).toBeDefined();
    const ep = list.episodes.find((e) => e.number === 42);
    expect(ep?.available).toContainEqual({ dubId: team!.id, maxQuality: 1080 });
    const streams = await ext.resolve(m, 42, team!.id, ctx);
    expect(streams[0]).toMatchObject({ kind: "hls", quality: 1080 });
    const playlist = await (await fetch(streams[0].url, { headers: streams[0].headers })).text();
    expect(playlist).toContain("#EXTINF");
  }, 120_000);
});
