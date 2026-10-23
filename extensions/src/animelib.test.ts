// Contract test against the live AnimeLib API. It bans heavy clients, so the live part stays small.
import { describe, expect, test } from "bun:test";
import * as ext from "./animelib";
import { makeCtx } from "./testing/host";

const ctx = makeCtx(ext.manifest.domains[0]);
const frieren = {
  shikimoriId: 52991,
  malId: 52991,
  names: ["Провожающая в последний путь Фрирен", "Sousou no Frieren"],
  year: 2023,
  kind: "tv",
  episodes: 28,
};
const FRIEREN = { id: "21371", title: "Фрирен", confidence: "exact" as const };

const play = (dub: string, name: string, type: "voice" | "subtitles" = "voice") => ({
  dub,
  name,
  type,
  player: "Kodik",
  src: "//kodikplayer.com/seria/1/abc/720p",
});

describe("animelib (offline)", () => {
  test("keys dubs by team and translation type", () => {
    const plays = ext.reducePlayers([
      { player: "Kodik", src: "//k/1", translation_type: { id: 2, label: "Озвучка" }, team: { id: 7, name: "AniDUB " } },
      { player: "Kodik", src: "//k/2", translation_type: { id: 1, label: "Субтитры" }, team: { id: 7, name: "AniDUB" } },
      { player: "Kodik", src: null, translation_type: { id: 2 }, team: { id: 8, name: "No src" } },
    ]);
    expect(plays.map((p) => [p.dub, p.name, p.type])).toEqual([
      ["7-v", "AniDUB", "voice"],
      ["7-s", "AniDUB", "subtitles"],
    ]);
  });

  test("numbers episodes, falling back to item_number when seasons restart", () => {
    expect(ext.numbered([{ id: 1, number: "1" }, { id: 2, number: "1.5" }, { id: 3, number: "2" }])).toEqual([
      { id: 1, n: 1 },
      { id: 3, n: 2 },
    ]);
    expect(
      ext.numbered([
        { id: 1, number: "1", item_number: 1 },
        { id: 2, number: "1", item_number: 2 },
      ]),
    ).toEqual([
      { id: 1, n: 1 },
      { id: 2, n: 2 },
    ]);
  });

  test("extends a team up to the last episode it was seen in, exact where players are known", () => {
    const eps = [1, 2, 3, 4].map((n) => ({ id: n * 10, n }));
    const a = play("1-v", "A");
    const b = play("2-v", "B");
    const s = play("3-s", "S", "subtitles");
    const list = ext.availability(eps, [[a, s], null, null, [a, b]]);
    expect(list.dubs.map((d) => d.id)).toEqual(["1-v", "3-s", "2-v"]);
    expect(list.episodes.map((e) => e.available.map((x) => x.dubId))).toEqual([
      ["1-v", "3-s"],
      ["1-v", "2-v"],
      ["1-v", "2-v"],
      ["1-v", "2-v"],
    ]);
    expect(list.episodes[3].available[0].maxQuality).toBe(720);
  });
});

describe("animelib (live)", () => {
  test("finds Frieren by shikimori id", async () => {
    const matches = await ext.search(frieren, ctx);
    expect(matches[0]).toMatchObject({ id: "21371", confidence: "exact" });
  }, 60_000);

  test("lists 28 episodes with teams and subtitles", async () => {
    const list = await ext.episodes(FRIEREN, ctx);
    expect(list.episodes.length).toBe(28);
    expect(list.dubs.length).toBeGreaterThan(10);
    expect(list.dubs.find((d) => d.name === "AniLibria.TV")?.type).toBe("voice");
    expect(list.dubs.some((d) => d.type === "subtitles")).toBe(true);
    const libria = list.dubs.find((d) => d.name === "AniLibria.TV")!;
    expect(list.episodes[27].available.map((a) => a.dubId)).toContain(libria.id);
  }, 60_000);

  test("resolves a Kodik team", async () => {
    const list = await ext.episodes(FRIEREN, ctx);
    const libria = list.dubs.find((d) => d.name === "AniLibria.TV")!;
    const streams = await ext.resolve(FRIEREN, 1, libria.id, ctx);
    expect(streams[0].kind).toBe("hls");
    expect(streams[0].url).toContain(".m3u8");
  }, 60_000);

  test("unknown episode is NotFound", async () => {
    await expect(ext.resolve(FRIEREN, 999, "32412-v", ctx)).rejects.toThrow("__EXT:NotFound");
  }, 60_000);
});
