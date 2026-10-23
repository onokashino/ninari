// Shikicinema: naming and sampling logic offline, a contract test against the live index and players.
import { describe, expect, test } from "bun:test";
import * as ext from "./shikicinema";
import { coverage, dubOf, nextProbes, teamOf } from "./shikicinema";
import { makeCtx } from "./testing/host";
import type { Ctx, Match } from "./types";

/**
 * GET with what the app's host adds: 10 s per request and one more try on a new connection. The Kinogram
 * CDN name resolves to several addresses and now and then some stall; the app's HTTP client moves on to
 * the next address, bun's fetch just hangs.
 */
async function get(url: string, init: { method?: string; headers?: Record<string, string>; body?: string } = {}): Promise<Response> {
  for (let attempt = 0; ; attempt++) {
    try {
      return await fetch(url, { ...init, signal: AbortSignal.timeout(10_000) });
    } catch (e) {
      if (attempt > 0) throw e;
    }
  }
}

/** makeCtx whose fetch goes through get(). */
function hostLike(baseUrl: string): Ctx {
  return {
    ...makeCtx(baseUrl),
    async fetch(url, init) {
      const r = await get(url, init);
      const body = await r.text();
      return { status: r.status, url: r.url, ok: r.ok, headers: Object.fromEntries(r.headers.entries()), text: async () => body, json: async () => JSON.parse(body) };
    },
  };
}

const frieren = {
  shikimoriId: 52991,
  malId: 52991,
  names: ["Провожающая в последний путь Фрирен", "Sousou no Frieren"],
  year: 2023,
  kind: "tv",
  episodes: 28,
};
const FRIEREN: Match = { id: "52991", title: "Frieren", confidence: "exact" };
const JJK: Match = { id: "40748", title: "Jujutsu Kaisen", confidence: "exact" };
const ONE_PIECE: Match = { id: "21", title: "One Piece", confidence: "exact" };

describe("shikicinema (offline)", () => {
  test("a team is the author without the cast in parentheses", () => {
    expect(teamOf("AniMedia (Shachiburi)")).toBe("AniMedia");
    expect(teamOf("DreamCast (Airis, Belka, Berserk)")).toBe("DreamCast");
    expect(teamOf("I.D.A. (Chokoba & ProjectSora)")).toBe("I.D.A.");
    expect(teamOf("AniMedia (Shachiburi) (277-278)")).toBe("AniMedia");
    expect(teamOf("277-278")).toBeUndefined();
    expect(teamOf("Часть 1")).toBeUndefined();
    expect(teamOf("(036-107) Фриза")).toBe("(036-107) Фриза");
    expect(teamOf("  Верт   &  Arimasu ")).toBe("Верт & Arimasu");
    expect(teamOf("Русская Озвучка | Оригинал")).toBeUndefined();
    expect(teamOf("Рус. Проф. многоголосый")).toBeUndefined();
    expect(teamOf(null)).toBeUndefined();
  });

  test("dubs by kind and team; unnamed subtitles and the original get a name, an unnamed voice-over is dropped", () => {
    expect(dubOf("озвучка", "AniDUB (Gecep)")).toEqual({ id: "v|anidub", name: "AniDUB", type: "voice" });
    expect(dubOf("субтитры", "Crunchyroll")).toEqual({ id: "s|crunchyroll", name: "Crunchyroll", type: "subtitles" });
    expect(dubOf("субтитры", null)).toEqual({ id: "s|субтитры", name: "Субтитры", type: "subtitles" });
    expect(dubOf("оригинал", "[JAP] RAW")).toEqual({ id: "o|оригинал", name: "Оригинал", type: "original" });
    expect(dubOf("озвучка", null)).toBeUndefined();
    expect(dubOf("трейлер", "x")).toBeUndefined();
    // Spellings of one team share an id.
    expect(dubOf("озвучка", "Anidub")?.id).toBe(dubOf("озвучка", "AniDUB")?.id);
  });

  test("samples the ends first, then the widest gaps, preferring gaps where a dub starts or stops", () => {
    const asked = (...ns: number[]) => Object.fromEntries(ns.map((n) => [String(n), [0, ["a"]]])) as Record<string, [number, string[]]>;
    expect(nextProbes({}, 1179, 5)).toEqual([1, 1179, 590, 295, 884]);
    expect(nextProbes(asked(1, 1179), 1179, 2)).toEqual([590, 295]);
    // After a play at 700 the next two episodes go first; stale latest episodes are asked again. 1 and 590
    // agree, so the stretch after 702 (not known yet) is split before theirs.
    expect(nextProbes(asked(1, 590, 1179), 1179, 4, 700, [1179])).toEqual([701, 702, 1179, 940]);
    // 1 and 590 agree, 590 and 1179 do not: the dub boundary is looked for first.
    const differ = { ...asked(1, 590), "1179": [0, ["b"]] } as Record<string, [number, string[]]>;
    expect(nextProbes(differ, 1179, 2)).toEqual([884, 1031]);
    expect(nextProbes(asked(1, 2, 3), 3, 5)).toEqual([]);
  });

  test("a dub found on two neighbouring samples covers the episodes between them", () => {
    const eps = { "1": [0, ["a", "b"]], "5": [0, ["a"]], "9": [0, ["a", "c"]] } as Record<string, [number, string[]]>;
    const c = coverage(eps, 9);
    expect([...c.keys()].sort((x, y) => x - y)).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect([...c.get(3)!]).toEqual(["a"]);
    expect([...c.get(1)!].sort()).toEqual(["a", "b"]);
    expect([...c.get(9)!].sort()).toEqual(["a", "c"]);
  });

  test("manifest reaches every player it lists", () => {
    expect(ext.manifest.domains).toEqual(["https://smarthard.net"]);
    expect(ext.manifest.allowedHosts).toEqual(
      expect.arrayContaining(["video.sibnet.ru", "ok.ru", "dzen.ru", "vk.com", "matreshka.tv", "*.kinogram.best", "*.okcdn.ru", "*.cmtv.ru"]),
    );
  });
});

describe("shikicinema (live)", () => {
  const ctx = hostLike(ext.manifest.domains[0]);

  test("finds a title by its Shikimori id", async () => {
    expect(await ext.search(frieren, ctx)).toEqual([{ id: "52991", title: frieren.names[0], confidence: "exact" }]);
    expect(await ext.search({ ...frieren, shikimoriId: 999999999 }, ctx)).toEqual([]);
  }, 30_000);

  test("lists Frieren's playable dubs only, Kinogram under the track it starts with", async () => {
    const list = await ext.episodes(FRIEREN, ctx);
    expect(list.episodes.length).toBe(28);
    expect(list.dubs).toContainEqual({ id: "v|studioband", name: "StudioBand", type: "voice" });
    expect(list.dubs).toContainEqual({ id: "s|субтитры", name: "Субтитры", type: "subtitles" });
    // Links on dead hosts (SovetRomantica, AnimeJoy, mail.ru...) and the generic Kinogram author are not offered.
    const names = list.dubs.map((d) => d.name);
    expect(names).not.toContain("SovetRomantica");
    expect(names).not.toContain("Русская Озвучка | Оригинал");
    expect(list.episodes[0].available).toContainEqual({ dubId: "v|studioband" });
  }, 60_000);

  test("resolves Frieren 1 in StudioBand through Kinogram", async () => {
    const [s] = await ext.resolve(FRIEREN, 1, "v|studioband", ctx);
    expect(s).toMatchObject({ kind: "hls", quality: 720 });
    const res = await get(s.url, { headers: s.headers });
    expect(res.status).toBe(200);
  }, 60_000);

  test("resolves Frieren 1 subtitles, skipping links that are gone", async () => {
    const [s] = await ext.resolve(FRIEREN, 1, "s|субтитры", ctx);
    expect(s.kind).toBe("hls");
    expect(s.quality).toBeGreaterThanOrEqual(720);
    const res = await get(s.url, { headers: s.headers });
    expect(res.status).toBe(200);
    expect(await res.text()).toStartWith("#EXTM3U");
  }, 60_000);

  test("plays Indie Dub on ok.ru for Jujutsu Kaisen", async () => {
    const list = await ext.episodes(JJK, ctx);
    expect(list.dubs).toContainEqual({ id: "v|indiedub", name: "Indie Dub", type: "voice" });
    const [s] = await ext.resolve(JJK, 14, "v|indiedub", ctx);
    expect(s.url).toContain("okcdn.ru");
    const res = await get(s.url, { headers: s.headers });
    expect(res.status).toBe(200);
  }, 60_000);

  test("samples One Piece instead of downloading 12 MB, and stays fast", async () => {
    const started = Date.now();
    const list = await ext.episodes(ONE_PIECE, ctx);
    expect(Date.now() - started).toBeLessThan(3000);
    expect(list.episodes.length).toBeGreaterThan(900);
    expect(list.dubs.map((d) => d.name)).toEqual(expect.arrayContaining(["AniMedia", "Get Smart Group", "Субтитры"]));
    const last = list.episodes[list.episodes.length - 1];
    expect(last.number).toBeGreaterThanOrEqual(1179);
    // Every call samples more episodes (the listing may shrink: a new sample can split a stretch a dub was
    // thought to cover).
    const sampled = async () => Object.keys(JSON.parse((await ctx.kv.get("probes:21"))!).eps).length;
    const before = await sampled();
    const again = await ext.episodes(ONE_PIECE, ctx);
    expect(await sampled()).toBeGreaterThanOrEqual(before + 10);
    expect(again.episodes.length).toBeGreaterThan(900);
  }, 60_000);

  test("resolves the latest One Piece episode with Crunchyroll subtitles in 1080p", async () => {
    const [s] = await ext.resolve(ONE_PIECE, 1179, "s|crunchyroll", ctx);
    expect(s).toMatchObject({ kind: "hls", quality: 1080 });
    const res = await get(s.url, { headers: s.headers });
    expect(res.status).toBe(200);
  }, 60_000);

  test("an episode or dub it does not have is NotFound", async () => {
    await expect(ext.resolve(FRIEREN, 999, "s|субтитры", ctx)).rejects.toThrow("__EXT:NotFound");
    await expect(ext.resolve(FRIEREN, 1, "v|nosuchteam", ctx)).rejects.toThrow("__EXT:NotFound");
  }, 60_000);
});
