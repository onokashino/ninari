// Contract test against the live AniLibria API.
import { describe, expect, test } from "bun:test";
import * as ext from "./anilibria";
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

describe("anilibria (live)", () => {
  test("finds Frieren by shikimori id", async () => {
    const matches = await ext.search(frieren, ctx);
    expect(matches[0]).toMatchObject({ id: "9542", confidence: "exact" });
  }, 30_000);

  test("lists 28 episodes in 1080p with the AniLibria dub", async () => {
    const list = await ext.episodes({ id: "9542", title: "Frieren", confidence: "exact" }, ctx);
    expect(list.dubs).toEqual([{ id: "aniliberty", name: "AniLibria", type: "voice" }]);
    expect(list.episodes.length).toBe(28);
    expect(list.episodes[0].available[0]).toEqual({ dubId: "aniliberty", maxQuality: 1080 });
  }, 30_000);

  test("resolves episode 1 with 1080p first", async () => {
    const streams = await ext.resolve({ id: "9542", title: "Frieren", confidence: "exact" }, 1, "aniliberty", ctx);
    expect(streams[0].quality).toBe(1080);
    expect(streams[0].kind).toBe("hls");
    expect(streams[0].url).toContain(".m3u8");
  }, 30_000);

  test("unknown episode is NotFound", async () => {
    await expect(
      ext.resolve({ id: "9542", title: "Frieren", confidence: "exact" }, 999, "aniliberty", ctx),
    ).rejects.toThrow("__EXT:NotFound");
  }, 30_000);
});
