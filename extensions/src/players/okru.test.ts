// ok.ru player: page parsing offline, resolution against the live site.
import { describe, expect, test } from "bun:test";
import { makeCtx } from "../testing/host";
import { isOkru, OKCDN_UA, okruId, okruStreams, OKRU_HOSTS, parseOkruPage, resolveOkru } from "./okru";

const ctx = makeCtx("https://ok.ru");
// Jujutsu Kaisen 14, Indie Dub: one of the few anime uploads on ok.ru that still play (Shikicinema, 2026-09).
const PLAYABLE = "https://ok.ru/videoembed/2744189454924";
// Frieren 1, subtitles: blocked on a rights holder's request, like most of them.
const BLOCKED = "https://ok.ru/videoembed/6260636977795";

const attr = (json: unknown) => JSON.stringify(json).replace(/&/g, "&amp;").replace(/"/g, "&quot;");

describe("okru (offline)", () => {
  test("recognizes embed forms", () => {
    expect(isOkru(PLAYABLE)).toBe(true);
    expect(isOkru("http://ok.ru/videoembed/707153037950")).toBe(true);
    expect(isOkru("//ok.ru/video/707153037950")).toBe(true);
    expect(isOkru("https://ok.ru/profile/123")).toBe(false);
    expect(isOkru("https://notok.ru/videoembed/1")).toBe(false);
    expect(okruId("http://ok.ru/videoembed/707153037950?autoplay=1")).toBe("707153037950");
    expect(OKRU_HOSTS).toContain("*.okcdn.ru");
  });

  test("reads the metadata from data-options, as an object or as a JSON string", () => {
    const metadata = {
      hlsManifestUrl: "https://vd1.okcdn.ru/video.m3u8?srcAg=CHROME&id=1",
      videos: [
        { name: "sd", url: "https://vd1.okcdn.ru/?type=2" },
        { name: "full", url: "https://vd1.okcdn.ru/?type=5" },
        { name: "hd", url: "https://vd1.okcdn.ru/?type=3", disallowed: true },
      ],
    };
    for (const md of [metadata, JSON.stringify(metadata)]) {
      const html = `<div data-module="OKVideo" data-options="${attr({ flashvars: { metadata: md } })}"></div>`;
      expect(parseOkruPage(html)).toEqual({ metadata });
    }
  });

  test("HLS first with the best MP4 as its quality, disallowed files skipped", () => {
    const h = { "User-Agent": OKCDN_UA };
    const streams = okruStreams(
      {
        hlsManifestUrl: "https://vd1.okcdn.ru/video.m3u8",
        videos: [
          { name: "sd", url: "https://vd1.okcdn.ru/?type=2" },
          { name: "full", url: "https://vd1.okcdn.ru/?type=5" },
          { name: "hd", url: "https://vd1.okcdn.ru/?type=3", disallowed: true },
        ],
      },
      h,
    );
    expect(streams.map((s) => [s.kind, s.quality])).toEqual([
      ["hls", 1080],
      ["mp4", 1080],
      ["mp4", 480],
    ]);
    expect(streams[0].headers?.["User-Agent"]).toBe(OKCDN_UA);
  });

  test("reports the stub a blocked video shows", () => {
    const html = `<div class="vp_video_stub_txt">Видео заблокировано из-за нарушений авторских прав</div>`;
    expect(parseOkruPage(html)).toEqual({ stub: "Видео заблокировано из-за нарушений авторских прав" });
    expect(parseOkruPage("<html></html>")).toBeNull();
  });
});

describe("okru (live)", () => {
  test("resolves a playable upload into HLS that answers with the browser agent", async () => {
    const streams = await resolveOkru(PLAYABLE, ctx);
    expect(streams[0].kind).toBe("hls");
    expect(streams[0].quality).toBeGreaterThanOrEqual(720);
    expect(streams[0].url).toContain("okcdn.ru");
    const res = await fetch(streams[0].url, { headers: streams[0].headers });
    expect(res.status).toBe(200);
    expect(await res.text()).toStartWith("#EXTM3U");
  }, 60_000);

  test("a blocked upload is NotFound", async () => {
    await expect(resolveOkru(BLOCKED, ctx)).rejects.toThrow("__EXT:NotFound");
  }, 60_000);
});
