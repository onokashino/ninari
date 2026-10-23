// Kinogram player: page parsing offline, resolution against the live site.
import { describe, expect, test } from "bun:test";
import { makeCtx } from "../testing/host";
import type { Ctx } from "../types";
import {
  isKinogram,
  KINOGRAM_HOSTS,
  kinogramAudio,
  masterHeight,
  parseKinogram,
  parseKinogramUrl,
  pickKinogram,
  resolveKinogram,
} from "./kinogram";

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

// Frieren 1 and Jujutsu Kaisen 23, as Shikicinema lists them (author "Русская Озвучка | Оригинал").
const FRIEREN = "https://api.kinogram.best/embed/kp/5401195?season=1&episode=1";
const JJK = "https://api.kinogram.best/embed/kp/1381125?season=1&episode=23";

const SERIES = `<script>makePlayer({ playlist: { id: 1, current: { season: 1, episode: "1" },
  seasons:[{"season":2,"blocked":false,"episodes":[{"episode":"1","hls":"https://a.interkh.com/s2e1/master.m3u8","audio":{"names":["Anilibria","Japan Original"],"order":[0,1]}}]},
  {"season":1,"blocked":false,"episodes":[{"episode":"1","hls":"https://a.interkh.com/s1e1/master.m3u8","audio":{"names":["AniBaza","StudioBand","Japan Original"],"order":[1,0,2]},"title":"x [1]"}]}]
  }, qualityByWidth: {"864":480} });</script>`;
const MOVIE = `<script>makePlayer({ hls: "https://a.interkh.com/movie/master.m3u8", audio: {"names":["Синема УС"],"order":[0]}, dash: "x" });</script>`;

describe("kinogram (offline)", () => {
  test("recognizes embed forms and reads the episode choice", () => {
    expect(isKinogram(FRIEREN)).toBe(true);
    expect(isKinogram("https://api1.kinogram.best/embed/imdb/123")).toBe(true);
    expect(isKinogram("https://kinogram.best/film/123")).toBe(false);
    expect(parseKinogramUrl(FRIEREN)).toEqual({ page: "https://api.kinogram.best/embed/kp/5401195", season: 1, episode: 1 });
    expect(parseKinogramUrl("https://api.kinogram.best/embed/kp/1")).toEqual({ page: "https://api.kinogram.best/embed/kp/1", season: undefined, episode: undefined });
    expect(KINOGRAM_HOSTS).toContain("*.interkh.com");
  });

  test("reads every season of a series, audio tracks in the master's order", () => {
    const page = parseKinogram(SERIES)!;
    expect(Object.keys(page.episodes).sort()).toEqual(["1:1", "2:1"]);
    expect(pickKinogram(page, { page: "", season: 1, episode: 1 })).toEqual({
      hls: "https://a.interkh.com/s1e1/master.m3u8",
      // "order" is the site player's choice; mpv starts on the master's first (DEFAULT) track.
      audio: ["AniBaza", "StudioBand", "Japan Original"],
    });
    expect(pickKinogram(page, { page: "", episode: 1 })?.hls).toContain("s1e1");
    expect(pickKinogram(page, { page: "", season: 3, episode: 1 })).toBeUndefined();
  });

  test("reads the single video of a movie", () => {
    const page = parseKinogram(MOVIE)!;
    expect(pickKinogram(page, { page: "" })).toEqual({ hls: "https://a.interkh.com/movie/master.m3u8", audio: ["Синема УС"] });
    expect(parseKinogram("<html></html>")).toBeNull();
  });

  test("reads the quality from the master playlist", () => {
    const m3u8 = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=640x360\na.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2,RESOLUTION=1280x720\nb.m3u8\n";
    expect(masterHeight(m3u8)).toBe(720);
    expect(masterHeight("#EXT-X-STREAM-INF:RESOLUTION=1920x800\n")).toBe(1080);
    expect(masterHeight("#EXTM3U\n")).toBeUndefined();
  });
});

describe("kinogram (live)", () => {
  test("names the audio track each title starts with", async () => {
    const ctx = hostLike("https://api.kinogram.best");
    expect((await kinogramAudio(FRIEREN, ctx))[0]).toBe("StudioBand");
    // The site's player switches to Anilibria, the master itself starts on the Japanese track.
    expect(await kinogramAudio(JJK, ctx)).toEqual(["Japan Original", "Рус. Проф. многоголосый", "Anilibria"]);
  }, 60_000);

  test("resolves an episode into a playable 720p master", async () => {
    const [s] = await resolveKinogram(FRIEREN, hostLike("https://api.kinogram.best"));
    expect(s).toMatchObject({ kind: "hls", quality: 720 });
    const res = await get(s.url);
    expect(res.status).toBe(200);
    expect(await res.text()).toContain("#EXT-X-MEDIA:TYPE=AUDIO");
  }, 60_000);

  test("an episode the page does not have is NotFound", async () => {
    const url = "https://api.kinogram.best/embed/kp/5401195?season=1&episode=99";
    await expect(resolveKinogram(url, hostLike("https://api.kinogram.best"))).rejects.toThrow("__EXT:NotFound");
  }, 60_000);
});
