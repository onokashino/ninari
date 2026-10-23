// Contract test against the live YummyAnime API and the players it embeds.
import { describe, expect, test } from "bun:test";
import * as ext from "./yummy";
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
const FRIEREN = { id: "10661", title: "Frieren", confidence: "exact" as const };
// Mo Dao Zu Shi: a donghua that Kodik carries with several dubs.
const MDZS = { id: "772", title: "Mo Dao Zu Shi", confidence: "exact" as const };

describe("yummy (live)", () => {
  test("finds Frieren by shikimori id", async () => {
    const matches = await ext.search(frieren, ctx);
    expect(matches[0]).toMatchObject({ id: "10661", confidence: "exact" });
  }, 30_000);

  test("falls back to a name search without ids", async () => {
    const matches = await ext.search({ ...frieren, shikimoriId: 0, malId: 0 }, ctx);
    expect(matches[0]).toMatchObject({ id: "10661", confidence: "fuzzy" });
  }, 30_000);

  test("works through the ru.yummyani.me mirror", async () => {
    const matches = await ext.search(frieren, makeCtx(ext.manifest.domains[1]));
    expect(matches[0]?.id).toBe("10661");
  }, 30_000);

  test("reports the quality CVH really has, not an assumed 1080p", async () => {
    const [match] = await ext.search(
      { shikimoriId: 61921, malId: 61921, names: ["Гробница богов 3"], year: 2025, kind: "ona", episodes: 52 },
      ctx,
    );
    const list = await ext.episodes(match, ctx);
    const anistar = list.episodes[0].available.find((a) => a.dubId.includes("AniStar"));
    expect(anistar?.maxQuality).toBe(2160);
  }, 60_000);

  test("lists 28 episodes with many dubs, subtitles kept apart", async () => {
    const list = await ext.episodes(FRIEREN, ctx);
    expect(list.episodes.length).toBe(28);
    expect(list.dubs.length).toBeGreaterThan(20);
    expect(list.dubs).toContainEqual({ id: "Озвучка AniLibria", name: "AniLibria", type: "voice" });
    expect(list.dubs).toContainEqual({ id: "Субтитры Crunchyroll", name: "Crunchyroll", type: "subtitles" });
    const first = list.episodes[0];
    expect(first.number).toBe(1);
    const anilibria = first.available.find((a) => a.dubId === "Озвучка AniLibria");
    expect(anilibria?.maxQuality).toBeGreaterThanOrEqual(720);
    expect(list.episodes[1].skip?.opening).toBeDefined();
  }, 60_000);

  test("resolves episode 1 in AniLibria into playable HLS", async () => {
    const streams = await ext.resolve(FRIEREN, 1, "Озвучка AniLibria", ctx);
    expect(streams[0].kind).toBe("hls");
    expect(streams[0].quality).toBeGreaterThanOrEqual(720);
    const res = await fetch(streams[0].url, { headers: streams[0].headers });
    expect(res.status).toBe(200);
    expect(await res.text()).toStartWith("#EXTM3U");
  }, 60_000);

  test("unknown episode is NotFound", async () => {
    await expect(ext.resolve(FRIEREN, 999, "Озвучка AniLibria", ctx)).rejects.toThrow("__EXT:NotFound");
  }, 60_000);

  test("donghua: finds Mo Dao Zu Shi and plays a Kodik dub", async () => {
    const matches = await ext.search({ shikimoriId: 37208, malId: 37208, names: ["Mo Dao Zu Shi"] }, ctx);
    expect(matches[0]).toMatchObject({ id: "772", confidence: "exact" });
    const list = await ext.episodes(MDZS, ctx);
    expect(list.episodes.length).toBe(15);
    const dub = list.episodes[0].available.find((a) => a.maxQuality === 720);
    expect(dub).toBeDefined();
    const streams = await ext.resolve(MDZS, 1, dub!.dubId, ctx);
    expect(streams[0].url).toContain(":hls:manifest.m3u8");
  }, 60_000);
});
