// Kodik player: cipher and page parsing offline, then live resolves against kodikplayer.com.
import { describe, expect, test } from "bun:test";
import { makeCtx } from "../testing/host";
import { decodeLink, isKodik, KODIK_HOSTS, parsePage, parseScript, qualityOf, resolveKodik } from "./kodik";

const ctx = makeCtx("https://kodikplayer.com");
const b64 = (s: string) => atob(s);

// Frieren, AniLibria dub, as YummyAnime hands it out (protocol-relative season embed).
const FRIEREN = "//kodikplayer.com/season/94795/d5a209e95d68094e346175d7357ef0c7/720p?translations=false&only_episode=true&only_season=true&episode=1";

const shiftLetters = (s: string, n: number) =>
  s.replace(/[a-zA-Z]/g, (c) => {
    const base = c <= "Z" ? 65 : 97;
    return String.fromCharCode(((c.charCodeAt(0) - base + n) % 26) + base);
  });

/** Encodes like Kodik does: base64 without padding, then letters shifted back by `shift`. */
const encode = (url: string, shift: number) => shiftLetters(btoa(url).replace(/=+$/, ""), 26 - shift);

describe("kodik (offline)", () => {
  const link = "//cloud.solodcdn.com/useruploads/351182fc/e27224cb:2026093016/720.mp4:hls:manifest.m3u8";

  test("decodes the 2025-2026 cipher (shift 18) and reports the shift", () => {
    expect(decodeLink(encode(link, 18), b64)).toEqual({ url: link, shift: 18 });
  });

  test("finds any other shift by brute force", () => {
    expect(decodeLink(encode(link, 7), b64)?.shift).toBe(7);
    expect(decodeLink(encode(link, 13), b64, 18)?.url).toBe(link);
  });

  test("decodes a live sample captured from /ftor", () => {
    const src =
      "Tg9rjO91HK5hj2fdHOVsjq5rj20dlFVtkvDejO9pHPUdUhCfUBogHuUbHBNpGg00VBQfTEM5HBUbUrGfUhIhHBG5Vro3T2CgVhQgVOVqVhtqHBM2VEM5HEGgHLDuHrQfULYfVEU2WrQeUrGeWBUeUBGdVhQeTu1eVLxwjPU6jENciEHtk3YcjBV1WI";
    expect(decodeLink(src, b64)?.url).toBe(
      "//cloud.solodcdn.com/useruploads/351182fc-e1ac-4521-a9e3-261303e69687/e27224cb79be165a9ef2d5ff210415c6:2026093016/720.mp4:hls:manifest.m3u8",
    );
  });

  test("passes plain links through and rejects garbage", () => {
    expect(decodeLink(link, b64)?.url).toBe(link);
    expect(decodeLink("!!!not-a-link!!!", b64)).toBeNull();
  });

  test("reads the endpoint and shift from the player script", () => {
    const js = 'x=1;$.ajax({type:"POST",url:atob("L2Z0b3I="),cache:!1});e.src=atob(t.replace(/[a-zA-Z]/g,function(e){return String.fromCharCode((e<="Z"?90:122)>=(e=e.charCodeAt(0)+18)?e:e-26)}))';
    expect(parseScript(js, b64)).toEqual({ path: "/ftor", shift: 18 });
    expect(parseScript("no ajax here", b64)).toBeNull();
  });

  test("parses urlParams and vInfo from the embed page", () => {
    const html = `<script>var urlParams = '{"d":"kodikplayer.com","d_sign":"a:1","pd":"kodikplayer.com","pd_sign":"b:1","ref":"","ref_sign":"c:1"}';</script>
      <script src="/assets/js/app.player_single.abc.js"></script>
      <script>vInfo.type = 'seria'; vInfo.hash = 'd5a2'; vInfo.id = '1211977';</script>`;
    expect(parsePage(html, "https://kodikplayer.com/season/94795/x/720p")).toEqual({
      params: { d: "kodikplayer.com", d_sign: "a:1", pd: "kodikplayer.com", pd_sign: "b:1", ref: "", ref_sign: "c:1" },
      type: "seria",
      hash: "d5a2",
      id: "1211977",
      script: "/assets/js/app.player_single.abc.js",
    });
    expect(parsePage("<html>Episode not found</html>", "https://kodikplayer.com/seria/1/x/720p")).toBeNull();
  });

  test("labels a stream by the file it points to", () => {
    // Old releases put 480.mp4 under the "720" key.
    expect(qualityOf("720", "https://cloud.solodcdn.com/x/e2:1/480.mp4:hls:manifest.m3u8")).toBe(480);
    expect(qualityOf("720", "https://cloud.solodcdn.com/x/e2:1/720.mp4:hls:manifest.m3u8")).toBe(720);
    expect(qualityOf("360", "https://cdn.example/x/stream.m3u8")).toBe(360);
    expect(qualityOf("auto", "https://cdn.example/x/stream.m3u8")).toBeUndefined();
  });

  test("recognizes Kodik embeds on every domain", () => {
    expect(isKodik(FRIEREN)).toBe(true);
    expect(isKodik("https://kodik.info/serial/73959/68e2e57cb95f7fb93655637acaca26c2/720p")).toBe(true);
    expect(isKodik("//aniqit.com/video/20609/e8fd5bc1190b7eb1ee1a3e1c3aec5f62/720p")).toBe(true);
    expect(isKodik("//video.sibnet.ru/shell.php?videoid=1")).toBe(false);
    expect(isKodik("https://kodikplayer.com/assets/js/app.js")).toBe(false);
  });

  test("allows the player and CDN hosts", () => {
    for (const h of ["*.kodikplayer.com", "*.kodik.info", "*.solodcdn.com"]) expect(KODIK_HOSTS).toContain(h);
  });
});

describe("kodik (live)", () => {
  test("resolves a protocol-relative season embed into 720/480/360 HLS", async () => {
    const streams = await resolveKodik(FRIEREN, ctx);
    expect(streams.map((s) => s.quality)).toEqual([720, 480, 360]);
    for (const s of streams) {
      expect(s.kind).toBe("hls");
      expect(s.url).toStartWith("https://");
      expect(s.url).toEndWith(`/${s.quality}.mp4:hls:manifest.m3u8`);
    }
    const res = await fetch(streams[0].url, { headers: streams[0].headers });
    expect(res.status).toBe(200);
    expect(await res.text()).toStartWith("#EXTM3U");
  }, 60_000);

  test("opens another episode of a season embed via options", async () => {
    const one = await resolveKodik(FRIEREN, ctx);
    const three = await resolveKodik(FRIEREN, ctx, { episode: 3 });
    expect(three[0].url).not.toBe(one[0].url);
  }, 60_000);

  test("rewrites dead legacy domains to kodikplayer.com", async () => {
    const streams = await resolveKodik(FRIEREN.replace("kodikplayer.com", "kodik.info"), ctx);
    expect(streams[0].quality).toBe(720);
  }, 60_000);

  test("a missing video is NotFound", async () => {
    await expect(
      resolveKodik("https://kodikplayer.com/seria/1211977/00000000000000000000000000000000/720p", ctx),
    ).rejects.toThrow("__EXT:NotFound");
  }, 60_000);
});
