// Contract test against the live Anixart API.
import { describe, expect, test } from "bun:test";
import * as ext from "./anixart";
import { makeCtx } from "./testing/host";

const ctx = makeCtx(ext.manifest.domains[0]);
const frieren = {
  shikimoriId: 52991,
  malId: 52991,
  names: ["Провожающая в последний путь Фрирен", "Sousou no Frieren", "Frieren: Beyond Journey's End"],
  year: 2023,
  kind: "tv",
  episodes: 28,
};
const FRIEREN = { id: "19081", title: "Фрирен", confidence: "exact" as const };
const ANILIBRIA = "2";

describe("anixart (offline)", () => {
  test("numbers Sibnet's zero-based positions from 1", () => {
    const list = ext.numbered([
      { position: 0, name: "1 серия", url: "a" },
      { position: 1, name: null, url: "b" },
      { position: 2, name: "3 серия", url: null },
    ]);
    expect(list).toEqual([
      { number: 1, url: "a" },
      { number: 2, url: "b" },
    ]);
  });

  test("reads per-episode qualities from a Libria iframe", () => {
    const html = `new Playerjs({id:"p", file:[{"id":"s1","file":"[480p]https:\\/\\/c\\/1\\/480.m3u8,[1080p]https:\\/\\/c\\/1\\/1080.m3u8"},{"id":"s2","file":"[720p]https:\\/\\/c\\/2.m3u8"}]});`;
    expect(ext.parseLibriaIframe(html, "https://anixart.libria.fun/public/iframe.php?id=1&ep=1")).toEqual([
      { url: "https://c/1/1080.m3u8", kind: "hls", quality: 1080 },
      { url: "https://c/1/480.m3u8", kind: "hls", quality: 480 },
    ]);
  });
});

describe("anixart (live)", () => {
  test("finds Frieren by title and year", async () => {
    const matches = await ext.search(frieren, ctx);
    expect(matches[0]).toMatchObject({ id: "19081", confidence: "exact" });
  }, 30_000);

  test("lists the dubs with 28 episodes", async () => {
    const list = await ext.episodes(FRIEREN, ctx);
    expect(list.dubs.length).toBeGreaterThan(15);
    expect(list.dubs).toContainEqual({ id: ANILIBRIA, name: "AniLibria", type: "voice" });
    expect(list.dubs.find((d) => d.name.startsWith("Субтитры"))?.type).toBe("subtitles");
    expect(list.episodes.length).toBe(28);
    expect(list.episodes[0].available).toContainEqual({ dubId: ANILIBRIA, maxQuality: 1080 });
  }, 60_000);

  test("resolves AniLibria through its own 1080p HLS", async () => {
    const streams = await ext.resolve(FRIEREN, 1, ANILIBRIA, ctx);
    expect(streams[0]).toMatchObject({ kind: "hls", quality: 1080 });
    expect(streams[0].url).toContain(".m3u8");
  }, 60_000);

  test("resolves a Sibnet-only dub into an mp4", async () => {
    // Air Gear: AniDUB (type 1) only has a Sibnet source, counted from position 0.
    const streams = await ext.resolve({ id: "100", title: "Air Gear", confidence: "exact" }, 1, "1", ctx);
    expect(streams[0]).toMatchObject({ kind: "mp4", headers: { Referer: "https://video.sibnet.ru/" } });
    expect(streams[0].url).toContain("video.sibnet.ru/v/");
  }, 60_000);

  test("resolves a Kodik dub", async () => {
    // Studio Band (type 29) only has Kodik.
    const streams = await ext.resolve(FRIEREN, 5, "29", ctx);
    expect(streams[0].kind).toBe("hls");
    expect(streams[0].url).toContain(".m3u8");
  }, 60_000);

  test("unknown episode is NotFound", async () => {
    await expect(ext.resolve(FRIEREN, 999, ANILIBRIA, ctx)).rejects.toThrow("__EXT:NotFound");
  }, 60_000);
});
