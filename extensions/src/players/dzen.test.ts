// Dzen player: page parsing offline, resolution against the live site.
import { describe, expect, test } from "bun:test";
import { makeCtx } from "../testing/host";
import { DZEN_HOSTS, dzenId, isDzen, parseDzen, resolveDzen } from "./dzen";

const ctx = makeCtx("https://dzen.ru");
// One Piece 1179, Crunchyroll subtitles, as Shikicinema lists it.
const EMBED = "https://dzen.ru/embed/oVEYCXEUOAAA?from_block=partner&from=zen&mute=1&autoplay=0&tv=0";
// Frieren 1: the upload was removed, Dzen answers 404.
const GONE = "https://dzen.ru/embed/v5-5GgMUeFAo?from_block=partner&from=zen&mute=1&autoplay=0&tv=0";

describe("dzen (offline)", () => {
  test("recognizes embed forms", () => {
    expect(isDzen(EMBED)).toBe(true);
    expect(isDzen("//dzen.ru/embed/v5-5GgMUeFAo")).toBe(true);
    expect(isDzen("https://dzen.ru/a/some-article")).toBe(false);
    expect(dzenId(EMBED)).toBe("oVEYCXEUOAAA");
    expect(DZEN_HOSTS).toContain("*.okcdn.ru");
  });

  test("HLS first with the best MP4 as its quality, DASH skipped", () => {
    const html = `<script>{"video":{"duration":1415,"streams":[
      {"url":"https://vd1.okcdn.ru/video.m3u8?id=1&dzen_dash=hls","type":"hls"},
      {"url":"https://vd1.okcdn.ru/?type=1&dzen_dash=dash","type":"dash"},
      {"url":"https://vd1.okcdn.ru/?type=2&dzen_dash=medium","type":"medium"},
      {"url":"https://vd1.okcdn.ru/?type=5&dzen_dash=fullhd","type":"fullhd"}]}}</script>`;
    const streams = parseDzen(html, {});
    expect(streams.map((s) => [s.kind, s.quality])).toEqual([
      ["hls", 1080],
      ["mp4", 1080],
      ["mp4", 480],
    ]);
    expect(streams[0].url).toContain("dzen_dash=hls");
    expect(parseDzen("<html></html>", {})).toEqual([]);
  });
});

describe("dzen (live)", () => {
  test("resolves an embed into HLS that answers with the browser agent", async () => {
    const streams = await resolveDzen(EMBED, ctx);
    expect(streams[0]).toMatchObject({ kind: "hls", quality: 1080 });
    const res = await fetch(streams[0].url, { headers: streams[0].headers });
    expect(res.status).toBe(200);
    expect(await res.text()).toStartWith("#EXTM3U");
  }, 60_000);

  test("a removed upload is NotFound", async () => {
    await expect(resolveDzen(GONE, ctx)).rejects.toThrow("__EXT:NotFound");
  }, 60_000);
});
